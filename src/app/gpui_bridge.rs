//! UI-agnostic bridge exposing `Tessel`'s shared backend state and actions to `crate::gpui_ui`.
//!
//! Ensures 100% zero-duplication state sharing between the primary `GPUI` renderer and the
//! `eframe`/`egui` Compatibility renderer.

use super::{Launch, Overlay, Session, SettingsSection, Store, Tessel, View, floating_terminal};
use crate::i18n::tr;
use crate::pane_grid::{FocusDir, PxRect, SplitDir};
use crate::theme_palettes::ThemeId;
use crate::workspace::{KeepAwakeSetting, RendererPreference, SideTab, Workspace};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::Duration,
};

pub(crate) type SettingsNavGroupRow = (String, Vec<(String, String)>);
pub(crate) type BuiltinAgentSettingRow = (String, String, bool, bool);
pub(crate) type ProviderSecretRow = (String, String, bool);
pub(crate) type ReviewBranchOption = (String, Option<PathBuf>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativePanelQueryScope {
    pub generation: u64,
    pub target: GpuiFocusTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeExplorerScope {
    pub generation: u64,
    pub owner: crate::git_result::Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExplorerCommand {
    Select,
    Open,
    NewFile,
    NewFolder,
    Rename,
    Trash,
    CopyPath,
    CopyRelative,
    Download,
    OpenEditor,
    ExternalEditor,
    InsertPath,
    TerminalHere,
    #[cfg(windows)]
    Reveal,
    Dotfiles,
    Compact,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NativeScmScope {
    pub owner: crate::git_result::Owner,
    pub id: u64,
}

#[derive(Clone, Debug)]
pub(crate) enum OutlineRowAction {
    Fold(String),
    SetAllFolds { expand: bool },
    Reveal(String, crate::outline_tree::Range, bool),
}

pub(crate) struct GpuiExtraUiState {
    outline_open: bool,
    outline_options_open: bool,
    outline_query: Option<(u64, String)>,
    disk_comparison_pending: bool,
    outline_cache: Option<crate::json_outline_panel::Cache>,
    pub dummy_egui_ctx: eframe::egui::Context,
    pub usage_popover_open: bool,
    pub toolbar_more_open: bool,
    pub layout_menu_right: Option<f32>,
    pub issues_menu_open: bool,
    pub notifications_popover_open: bool,
    pub status_popover: Option<crate::status_strip::StatusBarItemId>,
    pub launch_menu_open: bool,
    pub launch_placement: crate::finder_model::LaunchPlacement,
    pub launch_in_worktree: bool,
    pub modal_input_generation: u64,
    pub explorer_input_generation: u64,
    pub panel_query_generation: u64,
    pub session_query: crate::session_query::Search,
    pub conversation_model: crate::conversation_model::Model,
    pub conversation_loader: crate::conversation_loader::Loader,
    pub session_discovery: crate::session_discovery::Discovery,
    pub help_query: String,
    pub settings_search: String,
    pub scm_drafts: crate::scm_drafts::Drafts,
    pub new_task_title: String,
    pub task_board: crate::native_task_board::State,
    pub new_ssh_alias: String,
    pub new_ssh_folder: String,
    editing_ssh: Option<(usize, crate::workspace::Host)>,
    pub new_custom_agent_name: String,
    pub new_custom_agent_cmd: String,
    pub new_custom_agent_accent: usize,
    pub new_account_label: String,
    pub secret_key_idx: usize,
    pub secret_value_draft: String,
    pub skills_git_url: String,
    pub new_quick_cmd_name: String,
    pub new_quick_cmd_text: String,
    pub new_quick_cmd_enter: bool,
    pub new_auto_name: String,
    pub new_auto_prompt: String,
    pub new_auto_every_mins: u32,
    pub expanded_agent_id: Option<String>,
    pub copied_clipboard: Option<String>,
}

impl Default for GpuiExtraUiState {
    fn default() -> Self {
        Self {
            outline_open: false,
            outline_options_open: false,
            outline_query: None,
            disk_comparison_pending: false,
            outline_cache: None,
            dummy_egui_ctx: eframe::egui::Context::default(),
            usage_popover_open: false,
            toolbar_more_open: false,
            layout_menu_right: None,
            issues_menu_open: false,
            notifications_popover_open: false,
            status_popover: None,
            launch_menu_open: false,
            launch_placement: crate::finder_model::LaunchPlacement::Auto,
            launch_in_worktree: false,
            modal_input_generation: 1,
            explorer_input_generation: 1,
            panel_query_generation: 1,
            session_query: crate::session_query::Search::default(),
            conversation_model: crate::conversation_model::Model::default(),
            conversation_loader: crate::conversation_loader::Loader::default(),
            session_discovery: crate::session_discovery::Discovery::default(),
            help_query: String::new(),
            settings_search: String::new(),
            scm_drafts: crate::scm_drafts::Drafts::default(),
            new_task_title: String::new(),
            task_board: Default::default(),
            new_ssh_alias: String::new(),
            new_ssh_folder: String::new(),
            editing_ssh: None,
            new_custom_agent_name: String::new(),
            new_custom_agent_cmd: String::new(),
            new_custom_agent_accent: 0,
            new_account_label: String::new(),
            secret_key_idx: 0,
            secret_value_draft: String::new(),
            skills_git_url: String::new(),
            new_quick_cmd_name: String::new(),
            new_quick_cmd_text: String::new(),
            new_quick_cmd_enter: true,
            new_auto_name: String::new(),
            new_auto_prompt: String::new(),
            new_auto_every_mins: 60,
            expanded_agent_id: None,
            copied_clipboard: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum GpuiFocusTarget {
    #[default]
    None,
    Terminal(Option<u64>),
    TerminalFind(Option<u64>),
    ChatComposer(u64),
    ChatQuestion {
        pane_id: u64,
        q_idx: usize,
    },
    EditorFind,
    EditorReplace,
    EditorGoto,
    EditorBody,
    BrowserUrl,
    RemoteName,
    RemoteRename,
    RemoteText,
    PaletteQuery,
    FinderQuery,
    HelpQuery,
    ExplorerFilter,
    ExplorerTree,
    OutlineTree,
    OutlineQuery,
    ScmCommitMessage,
    TaskNewTitle,
    TaskRenameTitle,
    TaskDecisionAnswer,
    TaskCard(u64),
    TaskBoardControls,
    TaskBoardConfirm,
    DashboardQuery,
    TaskHistoryQuery,
    SessionsQuery,
    ConversationsQuery,
    SettingsSearch,
    NotesEditor,
    AddProjectCloneUrl,
    AddProjectFolderName,
    GitHubSearch,
    LiveShareJoinCode,
    GuestTaskDraft(crate::guest_task_board::Scope),
    PromptDraft,
    NewTeamName,
    QuickCommandName,
    QuickCommandText,
    CustomAgentName,
    CustomAgentCmd,
    AccountLabel,
    ProviderSecretValue,
    SkillsGitUrl,
    SshAlias,
    SshFolder,
    AutomationName,
    AutomationPrompt,
    BuiltinAgentCommand,
    BuiltinAgentArgs,
    BuiltinAgentEnv,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GpuiCenterMode {
    NoProjectsWelcome,
    EmptyProjectLauncher,
    WorkGrid,
    DocumentEditor,
    RemoteSftp,
    GuestFiles,
    BrowserTab,
}

#[derive(Clone, Debug)]
pub(crate) struct GpuiToolbarSnapshot {
    pub left_open: bool,
    pub right_open: bool,
    pub broadcast_enabled: bool,
    pub floating_open: bool,
    pub share_active: bool,
    pub share_pill_label: Option<String>,
    pub share_pill_tooltip: Option<String>,
    pub share_pill_badge: Option<String>,
    pub share_pill_dot_live: bool,
    pub more_menu_open: bool,
    pub layout_menu_right: Option<f32>,
    pub issues_menu_open: bool,
    pub has_layout_pane: bool,
    pub can_split_shell: bool,
    pub can_apply_grid: bool,
    pub can_arrange_panes: bool,
    pub waiting_agents_count: usize,
    pub first_waiting_pane: Option<u64>,
    pub unread_notifications: usize,
    pub has_active_project: bool,
    pub active_project_name: Option<String>,
    pub preferred_shell_label: String,
    pub installed_agents: Vec<(String, String)>,
    pub command_palette_open: bool,
    pub error_banner: Option<String>,
    pub build_label: String,
    pub dev_update_pill: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiSessionSidebarRow {
    pub id: u64,
    pub project_id: Option<u64>,
    pub title: String,
    pub kind_label: String,
    pub is_agent: bool,
    pub active: bool,
    pub sleeping: bool,
    pub waiting: bool,
    pub working: bool,
    pub exited: bool,
    pub worktree_branch: Option<String>,
    pub task_line: Option<String>,
    pub team_badge: Option<crate::teams_relay::SessionTeamBadge>,
    pub share_guests: Vec<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiWorktreeSidebarRow {
    pub branch: String,
    pub path: String,
    pub dirty: bool,
    pub hidden: bool,
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
    pub sessions: Vec<GpuiSessionSidebarRow>,
    pub visible_worktrees: Vec<GpuiWorktreeSidebarRow>,
    pub hidden_worktrees: Vec<GpuiWorktreeSidebarRow>,
    pub inbox_dismissed: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiSidebarSnapshot {
    pub prefs: crate::workspace::SidebarPreferences,
    pub collapsed_groups: BTreeSet<String>,
    pub orphan_sessions: Vec<GpuiSessionSidebarRow>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiViewTabItem {
    #[cfg(windows)]
    pub browser_id: Option<u64>,
    pub sound: Option<bool>,
    pub id: String,
    pub title: String,
    pub hint: String,
    pub kind: crate::view_tabs::Kind,
    pub doc_index: Option<usize>,
    pub guest_key: Option<crate::guest_file_editor::Key>,
    pub selected: bool,
    pub dirty: bool,
    pub closable: bool,
    pub preview: bool,
    pub deleted: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiPaneTileSnapshot {
    pub exit_banner: Option<(String, &'static str, &'static str)>,
    pub session_id: u64,
    pub title: String,
    pub kind_label: String,
    pub is_agent: bool,
    pub is_native_chat: bool,
    pub can_switch_chat_mode: bool,
    pub dormant: bool,
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
    pub terminal_render: Option<crate::terminal::TerminalRenderSnapshot>,
    pub cursor_style: crate::workspace::TerminalCursorStyle,
    pub native_chat: Option<crate::chat::NativeChatViewSnapshot>,
    pub team_badge: Option<crate::teams_relay::SessionTeamBadge>,
    pub share_guests: Vec<String>,
    pub wake_banner: Option<crate::teams_relay::WakeBanner>,
    pub error: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiFloatingTerminalSnapshot {
    pub open: bool,
    pub height_ratio: f32,
    pub title: String,
    pub starting: bool,
    pub error: Option<String>,
    pub terminal_render: Option<crate::terminal::TerminalRenderSnapshot>,
    pub cursor_style: crate::workspace::TerminalCursorStyle,
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
    pub listening_ports_count: usize,
    pub app_rss_mb: u64,
    pub version_build: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiActiveDocumentSummary {
    pub instance_id: u64,
    pub line_numbers: bool,
    pub syntax_highlighting: bool,
    pub doc_index: usize,
    pub external_available: bool,
    pub title: String,
    pub path_display: String,
    pub conflict_disk: Option<std::sync::Arc<str>>,
    pub comparison: Option<std::sync::Arc<crate::files::DiskComparison>>,
    pub deleted_on_disk: bool,
    pub disk_reload: u64,
    pub breadcrumb_path: Vec<String>,
    pub breadcrumb_symbols: std::sync::Arc<[crate::json_outline_panel::EditorBreadcrumb]>,
    pub extension: String,
    pub lang: crate::syntax::SyntaxLang,
    pub is_diff: bool,
    pub dirty: bool,
    pub preview_mode: bool,
    pub can_toggle_preview: bool,
    pub media: bool,
    pub word_wrap: bool,
    pub find_open: bool,
    pub replace_open: bool,
    pub goto_open: bool,
    pub find_query: String,
    pub match_case: bool,
    pub replace_text: String,
    pub goto_line: String,
    pub match_count: usize,
    pub match_index: usize,
    pub match_lines: Vec<usize>,
    pub hunk_lines: std::sync::Arc<[usize]>,
    pub scroll_to_line: Option<usize>,
    pub line_count: usize,
    pub text: std::sync::Arc<String>,
    pub force_large_markdown_render: bool,
    pub sheet: Option<std::sync::Arc<crate::rich_previews::SheetTable>>,
    pub json_preview: Option<crate::rich_previews::JsonDocumentPreview>,
    pub html_plain_text: Option<std::sync::Arc<str>>,
    pub readable_blocks: Vec<crate::preview::ReadableBlock>,
    pub pdf_summary: Option<crate::pdf::PdfSummary>,
    pub office_summary: Option<std::sync::Arc<crate::office::OfficeSummary>>,
    pub html_link_menu: Option<((f32, f32), bool)>,
    pub image_menu: Option<(f32, f32)>,
    pub image_download_notice: String,
    pub image_dims: Option<[usize; 2]>,
    pub playback: Option<crate::files::playback::Source>,
    pub media_pixels: Option<std::sync::Arc<gpui::RenderImage>>,
    pub error: String,
}

pub type DownloadSummaryRow = (u64, String, u64, Option<u64>, String, Option<String>);

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub(crate) struct GpuiBrowserSnapshot {
    pub loading_navigation: Option<u64>,
    pub retry_available: bool,
    pub failed_address: Option<String>,
    pub https_recovery_url: Option<String>,
    pub external_address: Option<String>,
    pub history: (bool, bool),
    pub tab_id: u64,
    pub started: bool,
    pub address: String,
    pub popup_url: Option<String>,
    pub downloads_open: bool,
    pub downloads: Vec<DownloadSummaryRow>,
    pub error: String,
    pub cookie_import_status: String,
}

#[cfg(windows)]
impl From<&crate::browser::Browser> for GpuiBrowserSnapshot {
    fn from(page: &crate::browser::Browser) -> Self {
        Self {
            tab_id: page.tab_id,
            history: page.history_directions(),
            started: page.is_started(),
            loading_navigation: page.loading_navigation(),
            https_recovery_url: page.https_recovery_url(),
            external_address: page.external_address(),
            retry_available: page.retry_available(),
            failed_address: page.failed_address().map(str::to_owned),
            address: page.display_address().to_owned(),
            popup_url: page.popup_url().map(str::to_owned),
            downloads_open: page.downloads_open(),
            downloads: page.download_summaries(),
            error: page.error_message(),
            cookie_import_status: page.cookie_import_status().to_owned(),
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExplorerPane {
    Outline,
    Timeline,
}

#[derive(Clone, Debug)]
pub(crate) struct ExplorerOpenEditor {
    pub title: String,
    pub folder: String,
    pub path: String,
    pub selected: bool,
    pub dirty: bool,
    pub preview: bool,
    pub select: GpuiAction,
    pub close: GpuiAction,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiSidePanelsSnapshot {
    pub dashboard_vm: crate::side_panels_model::DashboardViewModel,
    pub dashboard_group: crate::side_panels_model::DashboardGroupBy,
    pub dashboard_filter: Option<crate::side_panels_model::DashboardState>,
    pub dashboard_query: String,
    pub explorer: crate::explorer_tree::ExplorerGpuiSnapshot,
    pub open_editors: Vec<ExplorerOpenEditor>,
    pub editor_project: Option<u64>,
    pub open_editors_shown: bool,
    pub folder_shown: bool,
    pub outline_height: u16,
    pub timeline_height: u16,
    pub explorer_editable: bool,
    pub explorer_can_insert: bool,
    pub timeline: crate::file_timeline::Snapshot,
    pub outline_open: bool,
    pub outline_options_open: bool,
    pub outline_query: Option<String>,
    pub outline_filter_on_type: bool,
    pub outline_follow_cursor: bool,
    pub outline_selected: Option<String>,
    pub outline_scroll_generation: u64,
    pub outline_sort: crate::outline_tree::Sort,
    pub outline_folds: std::collections::HashSet<String>,
    pub outline_placeholder: &'static str,
    pub outline: Option<(u64, std::sync::Arc<Vec<crate::outline_tree::Symbol>>)>,
    pub scm_branch: Option<String>,
    pub scm_ahead: usize,
    pub scm_behind: usize,
    pub scm_busy: bool,
    pub scm_commit_busy: bool,
    pub scm_commit_msg: String,
    pub scm_entries: Vec<crate::scm_changes_panel::ScmEntry>,
    pub scm_error: Option<String>,
    pub commits_open: bool,
    pub commits: Vec<crate::side_panels_model::CommitHistoryRow>,
    pub expanded_commits: BTreeSet<String>,
    pub commit_files: BTreeMap<String, Vec<crate::side_panels_model::CommitChangedFile>>,
    pub tasks: Vec<crate::workspace::Task>,
    pub task_agents: Vec<crate::task_agents_model::Choice>,
    pub new_task_title: String,
    pub task_board: crate::native_task_board::State,
    pub history_window: crate::side_panels_model::TaskHistoryFilter,
    pub history_sort: crate::side_panels_model::TaskHistorySort,
    pub history_current_project_only: bool,
    pub history_query: String,
    pub task_history_rows: Vec<crate::side_panels_model::TaskHistoryRecord>,
    pub sessions_scope: crate::side_panels_model::SessionHistoryScope,
    pub sessions_group: crate::side_panels_model::SessionHistoryGroupBy,
    pub sessions_query: String,
    pub sessions_loading: bool,
    pub sessions_include_transcripts: bool,
    pub sessions_limited: bool,
    pub past_sessions: Vec<crate::side_panels_model::SessionHistoryEntry>,
    pub conv_window: crate::side_panels_model::ConversationWindow,
    pub conv_query: String,
    pub selected_thread_key: Option<String>,
    pub threads: std::sync::Arc<[crate::side_panels_model::ConversationThread]>,
    pub conversations_loading: bool,
    pub conversations_limited: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiBuiltinAgentDetail {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub enabled: bool,
    pub command: String,
    pub args: String,
    pub env: String,
    pub env_error: Option<String>,
    pub listed_models_caption: Option<String>,
    pub can_refresh_models: bool,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiSettingsSnapshot {
    pub active_page_id: String,
    pub search_query: String,
    pub groups: Vec<SettingsNavGroupRow>,
    pub general_prefs: crate::workspace::GeneralPreferences,
    pub preferred_shell: Option<String>,
    pub wsl_available: bool,
    pub terminal_prefs: crate::workspace::TerminalPreferences,
    pub editor_prefs: crate::workspace::EditorPreferences,
    pub browser_prefs: crate::workspace::BrowserPreferences,
    pub git_prefs: crate::workspace::GitPreferences,
    pub git_username: Option<String>,
    pub stats_prefs: crate::workspace::StatsPreferences,
    pub stats_snapshot: crate::settings_pages_panel::StatsSnapshot,
    pub usage_summary: crate::dialogs_toolbar_model::UsageSummary,
    pub quick_commands: Vec<crate::workspace::QuickCommand>,
    pub new_quick_cmd_name: String,
    pub new_quick_cmd_text: String,
    pub new_quick_cmd_enter: bool,
    pub can_import_ssh: bool,
    pub importing_ssh: bool,
    pub ssh_tests: std::collections::HashSet<String>,
    pub ssh_notice: Option<(String, bool)>,
    pub ssh_hosts: Vec<crate::workspace::Host>,
    pub new_ssh_alias: String,
    pub new_ssh_folder: String,
    pub editing_ssh: Option<(usize, crate::workspace::Host)>,
    pub builtin_agents: Vec<BuiltinAgentSettingRow>,
    pub builtin_agent_details: Vec<GpuiBuiltinAgentDetail>,
    pub agent_folder_trust: bool,
    pub expanded_agent_id: Option<String>,
    pub custom_agent_error: String,
    pub custom_agents: Vec<crate::settings_extra_model::CustomAgent>,
    pub new_custom_agent_name: String,
    pub new_custom_agent_cmd: String,
    pub new_custom_agent_accent: usize,
    pub claude_accounts: crate::settings_extra_model::ManagedAccountsRoster,
    pub codex_accounts: crate::settings_extra_model::ManagedAccountsRoster,
    pub claude_system_email: Option<String>,
    pub codex_system_email: Option<String>,
    pub new_account_label: String,
    pub provider_secrets: Vec<ProviderSecretRow>,
    pub secret_key_idx: usize,
    pub secret_value_draft: String,
    pub cli_sign_ins: crate::settings_extra_model::CliSignInProbe,
    pub skills: Vec<crate::settings_extra_model::SkillEntry>,
    pub skills_git_url: String,
    pub installed_mcp: Vec<crate::settings_extra_model::InstalledMcpServer>,
    pub mcp_probe_outcomes: BTreeMap<String, crate::settings_extra_model::McpProbeOutcome>,
    pub mcp_card_messages: BTreeMap<String, String>,
    pub mcp_test_status: String,
    pub context_prefs: crate::settings_extra_model::ContextPreferences,
    pub context_scan: crate::settings_extra_model::ContextScanReport,
    pub orchestration_prefs: crate::settings_extra_model::OrchestrationPreferences,
    pub notification_prefs: crate::settings_extra_model::NotificationPreferences,
    pub voice_prefs: crate::settings_extra_model::VoicePreferences,
    pub devtools: Vec<crate::settings_extra_model::DevToolStatus>,
    pub automations: crate::automations::AutomationsFile,
    pub new_auto_name: String,
    pub new_auto_prompt: String,
    pub new_auto_every_mins: u32,
    pub vault: Option<crate::vault_panel::VaultGpuiSnapshot>,
    pub account: Option<crate::account_panel::AccountGpuiSnapshot>,
    pub backup_status: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiPromptModalSnapshot {
    pub title: String,
    pub subtitle: String,
    pub draft: String,
    pub confirm_label: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiConfirmModalSnapshot {
    pub title: String,
    pub body: String,
    pub confirm_label: String,
    pub secondary_label: Option<String>,
    pub danger: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum EditorToolField {
    Find,
    Replace,
    Goto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum NativeSearchKind {
    Palette,
    Finder,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeSearchScope {
    pub kind: NativeSearchKind,
    pub generation: u64,
    pub project: Option<u64>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiModalsSnapshot {
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_selected: usize,
    pub palette_rows: Vec<crate::palette::PaletteRow>,
    pub finder_can_insert: bool,
    pub finder: Option<crate::finder_panel::FinderGpuiSnapshot>,
    pub launch_menu_open: bool,
    pub launch_placement: crate::finder_model::LaunchPlacement,
    pub launch_placements: Vec<crate::finder_model::LaunchPlacement>,
    pub launch_where_hint: String,
    pub launch_in_worktree: bool,
    pub help_open: bool,
    pub help_query: String,
    pub help_groups: Vec<crate::shortcuts::FilteredShortcutGroup>,
    pub help_footer: String,
    pub share_modal_open: bool,
    pub share: crate::share_panel::ShareGpuiSnapshot,
    pub add_project_open: bool,
    pub add_project_tab: crate::dialogs_toolbar_ui::AddProjectTab,
    pub add_picked_folder: Option<PathBuf>,
    pub add_nested_repos: Vec<(PathBuf, bool)>,
    pub add_clone_url: String,
    pub add_clone_parent: PathBuf,
    pub add_folder_name: String,
    pub add_ssh_aliases: Vec<String>,
    pub add_ssh_selected: String,
    pub add_ssh_remote_dir: String,
    pub add_error: String,
    pub github_open: bool,
    pub github_tab: crate::dialogs_toolbar_ui::GitHubTab,
    pub github_query: String,
    pub github_prs: Vec<crate::dialogs_toolbar_model::GhPullRequest>,
    pub github_issues: Vec<crate::dialogs_toolbar_model::GhIssue>,
    pub github_error: String,
    pub notes_open: bool,
    pub sessions_open: bool,
    pub notes_text: String,
    pub notes_dirty: bool,
    pub review_open: bool,
    pub review_candidates: Vec<ReviewBranchOption>,
    pub review_head_branch: String,
    pub review_snapshot: Option<crate::side_panels_model::ReviewSnapshot>,
    pub review_selected_file: Option<String>,
    pub review_selected_diff: String,
    pub review_error: Option<String>,
    pub worktree_cleanup: Option<crate::worktree_cleanup_panel::WorktreeCleanupModal>,
    pub teams_manager_project: Option<u64>,
    pub teams_new_name: String,
    pub teams_list: Vec<crate::teams_relay::SavedTeam>,
    pub teams_project_panes: Vec<(u64, String)>,
    pub teams_workers: Vec<crate::teams_relay::WorkerRecord>,
    pub teams_pending_workers: Vec<crate::teams_relay::WorkerRecord>,
    pub teams_relay_prompts: Vec<crate::teams_relay::RelayApprovalPrompt>,
    pub usage_popover_open: bool,
    pub usage_summary: crate::dialogs_toolbar_model::UsageSummary,
    pub notifications_popover_open: bool,
    pub notifications_items: Vec<crate::dialogs_toolbar_model::NotificationItem>,
    pub status_popover: Option<crate::status_strip::StatusBarItemId>,
    pub telemetry_app_rss_bytes: u64,
    pub telemetry_panes_rss_bytes: u64,
    pub telemetry_panes_cpu_pct: f32,
    pub telemetry_app_sparkline: Vec<f64>,
    pub telemetry_ports: Vec<crate::status_metrics::DetectedPort>,
    pub prompt_modal: Option<GpuiPromptModalSnapshot>,
    pub confirm_modal: Option<GpuiConfirmModalSnapshot>,
    pub dev_update_open: bool,
    pub dev_update_state: Option<crate::dev_update::DevUpdateState>,
    pub dev_update_busy: bool,
    pub dev_update_notice: Option<String>,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
pub(crate) struct HtmlPreviewToolbar {
    pub id: u64,
    pub page_path: String,
    pub decision: Option<bool>,
    pub reload: GpuiAction,
    pub open: GpuiAction,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) struct GpuiWorkspaceSnapshot {
    #[cfg(windows)]
    pub html_toolbar: Option<HtmlPreviewToolbar>,
    #[cfg(windows)]
    pub held_html_link: Option<(u64, url::Url)>,
    pub theme: ThemeId,
    pub file_icons: crate::theme_palettes::FileIconStyle,
    pub dark: bool,
    pub french: bool,
    pub font_size: f32,
    pub editor_font_size: u8,
    pub renderer: RendererPreference,
    pub side_browser: Option<GpuiBrowserSnapshot>,
    pub side_browser_tabs: Vec<(u64, String)>,
    pub selected_side_browser: Option<u64>,
    pub right_fullscreen: bool,
    pub right_tab: SideTab,
    pub center_mode: GpuiCenterMode,
    pub overlay_title: Option<String>,
    pub toolbar: GpuiToolbarSnapshot,
    pub sidebar: GpuiSidebarSnapshot,
    pub projects: Vec<GpuiProjectItem>,
    pub view_tabs: Vec<GpuiViewTabItem>,
    pub tiles: Vec<GpuiPaneTileSnapshot>,
    pub dividers: Vec<GpuiDividerSnapshot>,
    pub floating: GpuiFloatingTerminalSnapshot,
    pub active_document: Option<GpuiActiveDocumentSummary>,
    pub browser: GpuiBrowserSnapshot,
    pub remote: crate::remote_panel::control::Snapshot,
    pub side_panel: GpuiSidePanelSummary,
    pub side_panels: GpuiSidePanelsSnapshot,
    pub settings: Option<GpuiSettingsSnapshot>,
    pub modals: GpuiModalsSnapshot,
    pub status_bar: GpuiStatusBarSnapshot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HtmlViewMode {
    Source,
    Preview,
    Plain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddressCommand {
    Find,
    Back,
    Forward,
    Reload,
    HardReload,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SshRowCommand {
    Test,
    Disconnect,
    OpenTerminal,
    TogglePersistence,
    ToggleDaemon,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) enum GpuiAction {
    Remote(crate::remote_panel::control::Command),
    ToggleLeftSidebar,
    ToggleRightSidebar,
    SelectSideTab(SideTab),
    NewSideBrowser,
    DockBrowser(u64),
    UndockBrowser(u64),
    UndockBrowserOnPane {
        page: u64,
        pane: u64,
    },
    UndockBrowserAt {
        page: u64,
        anchor: Option<u64>,
        before: bool,
    },
    UndockBrowserToProject {
        page: u64,
        project: u64,
    },
    CloseSideBrowser(u64),
    SelectSideBrowser(u64),
    ToggleRightFullscreen,
    SelectProject(u64),
    AddProjectPath(PathBuf),
    PromptPickProjectFolder,
    OpenAddProjectDialog,
    NewShell,
    NewShellSplit(u64, SplitDir),
    SplitActiveShell(SplitDir),
    LaunchDefaultOrShell,
    LaunchAgent(String),
    LaunchCodexChat,
    LaunchClaudeChat,
    FocusSession(u64),
    CloseSession(u64),
    WakeSession(u64),
    RestartSession(u64),
    SwitchSessionChatMode(u64),
    ToggleMaximizeSession(u64),
    ToggleSessionBroadcast(u64),
    ToggleBroadcastMode,
    LayoutMenu(Option<f32>),
    ChooseLayout(crate::layout_broadcast_ui::LayoutMenuAction),
    MovePane {
        from: u64,
        to: Option<u64>,
        zone: crate::pane_grid::DropZone,
    },
    ResizeSplitDivider {
        split_id: String,
        dir: SplitDir,
        index: usize,
        split_side_px: f64,
        delta_px: f64,
    },
    SelectWorkTab,
    SelectDocumentTab(usize),
    KeepDocumentTab(usize),
    CloseDocumentTab(usize),
    SelectBrowserTab,
    #[cfg(windows)]
    SelectBrowserPage(u64),
    #[cfg(windows)]
    CloseBrowserPage(u64),
    ToggleBrowserSound(u64),
    CloseBrowserTab,
    ToggleFloatingTerminal,
    RestartFloatingTerminal,
    CloseFloatingTerminal,
    SetFloatingHeightRatio(f32),
    // Terminal actions (`None` targets the Floating Terminal)
    TerminalScroll {
        session_id: Option<u64>,
        delta_lines: i64,
    },
    TerminalBackToLive(Option<u64>),
    TerminalPaste {
        session_id: Option<u64>,
        text: String,
    },
    TerminalConfirmMultilinePaste {
        session_id: Option<u64>,
        confirm: bool,
    },
    TerminalOpenFind(Option<u64>),
    TerminalCloseFind(Option<u64>),
    ApplyNativeTerminalFindEdit {
        session_id: Option<u64>,
        instance_id: u64,
        base: String,
        text: String,
    },
    NativeTerminalFindCommand {
        session_id: Option<u64>,
        instance_id: u64,
        query: String,
        step: Option<isize>,
    },
    TerminalDismissDeviceLogin(Option<u64>),
    TerminalOpenTesselUrl(String),
    // Native Chat (Codex & Claude) actions
    ChatConnect(u64),
    ChatStop(u64),
    ChatInterrupt(u64),
    ChatNewConversation(u64),
    ChatLoadOlder(u64),
    ApplyNativeChatEdit {
        pane_id: u64,
        instance_id: u64,
        base: String,
        text: String,
    },
    NativeChatSend {
        pane_id: u64,
        instance_id: u64,
        text: String,
    },
    ChatSetModel {
        pane_id: u64,
        model: String,
    },
    ChatSetEffort {
        pane_id: u64,
        effort: String,
    },
    ChatSetResume {
        pane_id: u64,
        resume: String,
    },
    ChatRespondApproval {
        pane_id: u64,
        request_instance_id: u64,
        allow: bool,
    },
    ChatSelectQuestionOption {
        pane_id: u64,
        request_instance_id: u64,
        q_idx: usize,
        opt_idx: usize,
    },
    ApplyNativeQuestionEdit {
        pane_id: u64,
        request_instance_id: u64,
        q_idx: usize,
        base: String,
        text: String,
    },
    ChatSubmitQuestions {
        pane_id: u64,
        request_instance_id: u64,
        skip: bool,
    },
    // Document / Diff / Rich Preview actions
    OpenChatCitation(u64, crate::chat::citations::FileLink),
    OpenDocumentPath(PathBuf),
    ResolveDiskConflict(u64, std::sync::Arc<str>, crate::files::ConflictAction),
    SaveActiveDocument,
    OpenEditorExternally(u64),
    SaveGuestDocument(crate::guest_file_editor::Key),
    RecoverGuestFile(
        crate::guest_file_editor::Key,
        crate::share_panel::guest_files::RecoveryAction,
    ),
    CopyGuestFileText(crate::guest_file_editor::Key),
    SelectGuestDocument(crate::guest_file_editor::Key),
    #[cfg(windows)]
    OpenHtmlInBrowser(u64),
    #[cfg(windows)]
    OpenRemoteHtmlInBrowser {
        host: String,
        path: String,
    },
    #[cfg(windows)]
    ReloadHtmlPreview(u64),
    SetHtmlView(u64, HtmlViewMode),
    ToggleActiveDocumentPreview,
    ForceRenderLargeMarkdown,
    ToggleEditorWordWrap,
    ToggleEditorFind,
    ToggleEditorReplace,
    ToggleEditorGoto,
    SetEditorFindQuery(String),
    ToggleEditorMatchCase,
    SetEditorReplaceText(String),
    StepEditorFind(isize),
    ReplaceEditorCurrent,
    ReplaceEditorAll,
    SetEditorGotoSpec(String),
    ApplyEditorGoto,
    EditActiveDocumentLine {
        line_idx: usize,
        new_line: String,
    },
    SelectNativeEditorMatch {
        instance_id: u64,
        base: String,
        query: String,
        from_char: usize,
    },
    ApplyNativeEditorToolEdit {
        instance_id: u64,
        field: EditorToolField,
        base: String,
        text: String,
    },
    ApplyNativeDocumentEdit {
        instance_id: u64,
        base: String,
        text: String,
    },
    ImageMenu(u64, Option<(f32, f32)>),
    DownloadImage(u64),
    PdfFit(u64),
    StepPdfZoom(u64, f32),
    SelectOfficeSection(usize),
    HtmlLinkMenu(u64, String, (f32, f32)),
    CloseHtmlLinkMenu,
    #[cfg(windows)]
    AskHtmlTrust(u64),
    #[cfg(windows)]
    DecideHtmlTrust(u64, bool),
    #[cfg(windows)]
    ResolveHtmlExternal(u64, url::Url, bool),
    OpenHtmlLinkMenuTarget(u64),
    FollowDocumentLink(u64, String),
    // Embedded Browser Tab actions
    BrowserSetAddress(String),
    BeginNativeBrowserAddress((u64, Option<u64>)),
    CancelNativeBrowserAddress((u64, Option<u64>)),
    BrowserAddressCommand {
        scope: (u64, Option<u64>),
        command: AddressCommand,
    },
    NativeBrowserAddress {
        scope: (u64, Option<u64>),
        base: String,
        text: String,
        navigate: bool,
    },
    BrowserNavigate,
    SideBrowserBack(u64),
    SideBrowserForward(u64),
    SideBrowserReload(u64),
    BrowserBack(u64),
    BrowserForward(u64),
    BrowserReload(u64),
    BrowserReloadMenu(u64),
    BrowserTryHttps(u64, String),
    BrowserStop {
        tab: u64,
        navigation: u64,
    },
    BrowserImportCookies,
    BrowserAcceptPopup,
    BrowserDismissPopup,
    BrowserToggleDownloads(u64),
    BrowserCancelDownload {
        tab: u64,
        id: u64,
    },
    BrowserRevealDownload {
        tab: u64,
        id: u64,
    },
    // Sidebar (`ProjectAction`) & Options
    Project(crate::project_panel::ProjectAction),
    SetSidebarPrefs(crate::workspace::SidebarPreferences),
    // SidePanel 7 Tabs (Dashboard, Explorer, SCM Changes + Commits, Tasks, Runs, Sessions, Threads)
    SidePanels(crate::side_panels_ui::SidePanelsAction),
    SetDashboardGroup(crate::side_panels_model::DashboardGroupBy),
    SetDashboardFilter(Option<crate::side_panels_model::DashboardState>),
    SetDashboardQuery(String),
    NativePanelQueryEdit {
        scope: NativePanelQueryScope,
        base: String,
        text: String,
    },
    NativePanelQueryExitFullscreen {
        scope: NativePanelQueryScope,
        base: String,
    },
    ExplorerSetFilter(String),
    NativeExplorerEdit {
        scope: NativeExplorerScope,
        base: String,
        text: String,
    },
    ExplorerToggleSearchMode,
    ExplorerSetSearchMode(bool),
    ExplorerToggleFolder,
    ExplorerPaneHeight(ExplorerPane, u16),
    ExplorerCollapseFolders,
    ExplorerToggleEditors,
    ExplorerActivateDocument(u64),
    ExplorerCloseDocument(u64),
    ExplorerSaveAll(u64),
    ExplorerCloseAll(u64),
    ExplorerToggleDotfiles,
    ExplorerToggleCompact,
    Timeline(crate::file_timeline::Action),
    RevealBreadcrumb(u64, usize, crate::outline_tree::Range),
    OutlineToggle,
    OutlineOptions(bool),
    OutlineFilterMode(bool),
    OutlineStartQuery(u64, String),
    OutlineCloseQuery(u64),
    OutlineNavigate(u64, eframe::egui::Key),
    OutlineSort(crate::outline_tree::Sort),
    OutlineFollow(bool),
    OutlineRow(
        u64,
        std::sync::Arc<Vec<crate::outline_tree::Symbol>>,
        OutlineRowAction,
    ),
    ExplorerRefresh,
    ExplorerMenu(NativeExplorerScope, Option<String>, ExplorerCommand),
    ExplorerNavigate(PathBuf, eframe::egui::Key),
    ExplorerClickRow {
        key: String,
        pinned: bool,
    },
    ExplorerOpenContentHit {
        path: PathBuf,
        line: usize,
    },
    ExplorerNewItem {
        dir: bool,
    },
    ExplorerDeletePath {
        path: PathBuf,
        permanent: bool,
    },
    ScmRefresh,
    ScmSetCommitMessage(String),
    NativeScmEdit {
        scope: NativeScmScope,
        base: String,
        text: String,
    },
    NativeScmTouch {
        scope: NativeScmScope,
        base: String,
    },
    NativeScmSubmit {
        scope: NativeScmScope,
        base: String,
    },
    ScmCommit,
    ScmStage(Vec<String>),
    ScmUnstage(Vec<String>),
    ScmRequestDiscard {
        area: crate::scm_changes_panel::ScmArea,
        paths: Vec<String>,
    },
    ScmOpenDiff {
        area: crate::scm_changes_panel::ScmArea,
        path: String,
        old_path: Option<String>,
        pinned: bool,
    },
    ScmRemoteOp(crate::scm_changes_panel::ScmRemoteOp),
    ScmToggleCommitsOpen,
    OpenNotesModal,
    SetNotesText(String),
    OpenReviewModal,
    TasksSetNewTitle(String),
    TasksAddFromDraft,
    NativeTaskSubmit {
        scope: NativePanelQueryScope,
        base: String,
    },
    TasksBoard(crate::tasks_board_panel::TasksBoardAction),
    NativeTaskBoard {
        scope: crate::native_task_board::Scope,
        command: crate::native_task_board::Command,
    },
    SetTaskHistoryWindow(crate::side_panels_model::TaskHistoryFilter),
    SetTaskHistorySort(crate::side_panels_model::TaskHistorySort),
    ToggleTaskHistoryCurrentProject,
    SetTaskHistoryQuery(String),
    SetSessionsScope(crate::side_panels_model::SessionHistoryScope),
    SetSessionsGroup(crate::side_panels_model::SessionHistoryGroupBy),
    SetSessionsQuery(String),
    ToggleSessionsTranscripts,
    SetConversationsWindow(crate::side_panels_model::ConversationWindow),
    SetConversationsQuery(String),
    SelectConversationThread(Option<String>),
    // Settings (22 pages + Windows Vault & Tessel Account)
    SetSettingsSearch(String),
    SettingsPage(crate::settings_pages_panel::SettingsPageAction),
    SettingsExtra(crate::settings_extra_panel::SettingsExtraAction),
    UpdateGeneralPrefs(crate::workspace::GeneralPreferences),
    UpdateTerminalPrefs(crate::workspace::TerminalPreferences),
    UpdateEditorPrefs(crate::workspace::EditorPreferences),
    SetEditorFontSize {
        base: u32,
        size: u32,
    },
    SetAutoSaveDelay {
        base: u32,
        delay: u32,
    },
    UpdateBrowserPrefs(crate::workspace::BrowserPreferences),
    UpdateGitPrefs(crate::workspace::GitPreferences),
    UpdateStatsPrefs(crate::workspace::StatsPreferences),
    UpdateContextPrefs(crate::settings_extra_model::ContextPreferences),
    UpdateOrchestrationPrefs(crate::settings_extra_model::OrchestrationPreferences),
    UpdateNotificationPrefs(crate::settings_extra_model::NotificationPreferences),
    UpdateVoicePrefs(crate::settings_extra_model::VoicePreferences),
    ToggleBuiltinAgentEnabled(String),
    SetQuickCommandDraft {
        name: String,
        text: String,
        press_enter: bool,
    },
    SubmitNewQuickCommand,
    SetSshDraft {
        alias: String,
        folder: String,
    },
    ImportSshConfig,
    SshRow(usize, crate::workspace::Host, SshRowCommand),
    BeginSshEdit(usize, crate::workspace::Host),
    CancelSshEdit,
    SaveSshHostFromDraft,
    RemoveSshHost(usize, crate::workspace::Host),
    ToggleAgentFolderTrust,
    SetCustomAgentDraft {
        name: String,
        cmd: String,
    },
    SetCustomAgentAccent(usize),
    SubmitNewCustomAgent,
    SetAccountLabelDraft(String),
    SelectSecretKeyIndex(usize),
    SetSecretValueDraft(String),
    SaveSecretFromDraft,
    SetSkillsGitUrlDraft(String),
    SetAutomationDraft {
        name: String,
        prompt: String,
        every_mins: u32,
    },
    SubmitNewAutomation,
    VaultUnlock,
    VaultImportCsv,
    VaultDeleteEntry(String),
    VaultFillEntry(String),
    AccountOpenStore,
    AccountSignInBrowser,
    AccountRefresh,
    AccountOpenAdminPortal,
    AccountSignOut,
    AccountSuggestPhrase,
    ToggleExpandBuiltinAgent(String),
    RefreshBuiltinAgentModels(String),
    ResetBuiltinAgentPrefs(String),
    DismissMcpCardMessage(String),
    ToggleDevUpdateDialog,
    RefreshDevUpdate,
    ApplyDevUpdate,
    CopyGitPullCommand,
    // Modals, Popovers & Navigation
    NativeSearchEdit {
        scope: NativeSearchScope,
        base: String,
        text: String,
    },
    NativeSearchStep {
        scope: NativeSearchScope,
        delta: isize,
    },
    NativeSearchAccept {
        scope: NativeSearchScope,
        query: String,
        choice: Option<String>,
        insert: bool,
    },
    ToggleCommandPalette,
    PaletteSetQuery(String),
    PaletteStep(isize),
    PaletteExecute(String),
    PaletteExecuteSelected,
    OpenFinder,
    FinderSetQuery(String),
    FinderStep(isize),
    FinderAccept {
        idx: Option<usize>,
        insert: bool,
    },
    ToggleLaunchMenu,
    SetLaunchPlacement(crate::finder_model::LaunchPlacement),
    ToggleLaunchInWorktree,
    LaunchFromMenu {
        agent_id: Option<String>,
        native_chat: Option<&'static str>,
    },
    OpenSettings,
    OpenSettingsPage(String),
    CloseOverlays,
    OpenHelp,
    SetHelpQuery(String),
    NativeModalQuery {
        scope: (u64, GpuiFocusTarget),
        base: String,
        text: String,
    },
    DismissNativeModalQuery {
        scope: (u64, GpuiFocusTarget),
        base: String,
    },
    OpenLiveShare,
    ToggleToolbarMore,
    ToggleIssuesMenu,
    ToolbarMore(crate::toolbar_fit::MoreMenuAction),
    Share(crate::share_panel::ShareGpuiAction),
    OpenGitHubDialog,
    OpenLinearDialog,
    DialogsToolbar(crate::dialogs_toolbar_ui::DialogsToolbarAction),
    SetAddProjectTab(crate::dialogs_toolbar_ui::AddProjectTab),
    ToggleNestedRepo(PathBuf),
    SetAddProjectCloneUrl(String),
    SetAddProjectFolderName(String),
    SetAddProjectSshAlias(String),
    SetGitHubTab(crate::dialogs_toolbar_ui::GitHubTab),
    SetGitHubQuery(String),
    OpenTeamsManager(u64),
    CloseTeamsManager,
    SetNewTeamName(String),
    Teams(crate::teams_ui::TeamsUiAction),
    WorktreeCleanupTogglePath(PathBuf),
    WorktreeCleanupSelectSafe,
    WorktreeCleanupSelectNone,
    WorktreeCleanupConfirmRemove,
    WorktreeCleanupClose,
    ToggleUsagePopover,
    ToggleNotificationsPopover,
    ToggleStatusPopover(crate::status_strip::StatusBarItemId),
    RefreshPortsNow,
    SetPromptDraft(String),
    SubmitPromptModal,
    CancelPromptModal,
    ConfirmModalPrimary,
    ConfirmModalSecondary,
    CancelConfirmModal,
    SetTheme(ThemeId),
    SetFileIcons(crate::theme_palettes::FileIconStyle),
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

fn apply_singleline_edit(
    cur: &mut String,
    lower: &str,
    printable: Option<&str>,
    pasted: Option<String>,
) -> bool {
    if let Some(p) = pasted {
        let single = p.replace(['\r', '\n'], " ");
        cur.push_str(&single);
        true
    } else if lower == "backspace" {
        let _ = cur.pop();
        true
    } else if let Some(txt) = printable {
        cur.push_str(txt);
        true
    } else {
        false
    }
}

impl Tessel {
    pub(crate) fn new_for_gpui(repaint_cb: impl Fn() + Send + Sync + 'static) -> Self {
        let override_dir = std::env::args()
            .collect::<Vec<_>>()
            .windows(2)
            .find(|pair| pair[0] == "--data-dir")
            .map(|pair| PathBuf::from(&pair[1]));
        #[cfg(feature = "visual-test")]
        if override_dir.is_some() && std::env::args().any(|arg| arg == "--remote-fixture") {
            let fixture_dir = override_dir.clone().unwrap();
            let mut app = Self::new_for_gpui_in_dir(override_dir, false, repaint_cb);
            app.view = View::Remote;
            app.remote = crate::remote_panel::RemotePanel::text_fixture();
            if std::env::args().any(|arg| arg == "--remote-pdf-fixture") {
                app.remote = crate::remote_panel::RemotePanel::pdf_fixture();
            } else if std::env::args().any(|arg| arg == "--remote-image-fixture") {
                app.remote = crate::remote_panel::RemotePanel::fixture();
            }
            if std::env::args().any(|arg| arg == "--french-fixture") {
                app.state.language = crate::i18n::Language::French;
            }
            if std::env::args().any(|arg| arg == "--media-fixture") {
                let setup = (|| -> Result<(), String> {
                    use std::io::Write;
                    let path = fixture_dir.join("synthetic-page.pdf");
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map_err(|e| e.to_string())?;
                    file.write_all(&crate::pdf::fixture())
                        .map_err(|e| e.to_string())?;
                    app.documents
                        .push(crate::files::Document::open(&fixture_dir, &path)?);
                    let path = fixture_dir.join("synthetic-colors.png");
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .map_err(|e| e.to_string())?;
                    let pixels = image::RgbaImage::from_fn(320, 180, |x, _| {
                        image::Rgba(if x < 160 {
                            [255, 0, 0, 255]
                        } else {
                            [0, 0, 255, 255]
                        })
                    });
                    pixels
                        .write_to(&mut std::io::BufWriter::new(file), image::ImageFormat::Png)
                        .map_err(|e| e.to_string())?;
                    app.documents
                        .push(crate::files::Document::open(&fixture_dir, &path)?);
                    Ok(())
                })();
                if let Err(error) = setup {
                    app.error = error;
                }
            }
            return app;
        }
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
            Ok((s, w)) => {
                let warning = crate::i18n::translate(w.language, &s.warning).to_owned();
                (Some(s), w, warning)
            }
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
        let notifications = super::notification_inbox::load(store.as_ref());
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
        let remote = crate::remote_panel::RemotePanel::with_profile(
            store.as_ref().map(|s| s.directory().to_owned()),
        );
        #[cfg(windows)]
        let browser_project = state.active_project;
        #[cfg(windows)]
        let browser = crate::browser::Browser::default();
        #[cfg(windows)]
        let editor_page = crate::browser::editor_page::EditorPage::new(&browser);
        let mut app = Self {
            #[cfg(windows)]
            window_frame: None,
            floating: floating_terminal::State::new_with_repaint(floating_repaint, state.floating),
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
            editor_match_case: false,
            document_watch: Default::default(),
            next_document_request: 0,
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
            claude_system_email: if probe_environment {
                directories::BaseDirs::new().and_then(|b| {
                    crate::settings_extra_model::probe_system_provider_email(b.home_dir(), "claude")
                })
            } else {
                None
            },
            codex_system_email: if probe_environment {
                directories::BaseDirs::new().and_then(|b| {
                    crate::settings_extra_model::probe_system_provider_email(b.home_dir(), "codex")
                })
            } else {
                None
            },
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
            retire_old_google: Default::default(),
            dev_update: crate::dev_update::DevUpdatePoller::new(probe_environment),
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
            host_tests: Default::default(),
            host_import: None,
            ssh_sync_home: probe_environment
                .then(directories::BaseDirs::new)
                .flatten()
                .map(|base| base.home_dir().to_owned()),
            ssh_settings_visible: false,
            tasks_board: Default::default(),
            tasks_board_project: None,
            pending_close: None,
            remaining_close_requests: Vec::new(),
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
            #[cfg(windows)]
            pending_browser_launch: None,
            worktree_state: Default::default(),
            worktree_cleanup: None,
            pending_delete_worktree: None,
            git_username: None,
            git_username_rx: if probe_environment {
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
            } else {
                None
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
            status_telemetry: crate::status_metrics::StatusTelemetry::spawn(telemetry_repaint),
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
            browser_project,
            #[cfg(windows)]
            background_browsers: Default::default(),
            #[cfg(windows)]
            other_browser_pages: Default::default(),
            #[cfg(windows)]
            side_browsers: Default::default(),
            #[cfg(windows)]
            selected_side_browser: None,
            #[cfg(windows)]
            side_browser_history: Default::default(),
            #[cfg(windows)]
            finishing_browser_pages: Vec::new(),
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
            #[cfg(windows)]
            app.restore_owner_browser_layout();
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
            app.active_session = app.active_project_work_pane();
        }
        if probe_environment {
            app.detect_agents();
            app.refresh_project();
            app.worktree_state.refresh_all(&app.state.projects);
        }
        app
    }

    pub(crate) fn poll_for_gpui(&mut self) -> bool {
        // These workers use channels rather than the terminal repaint callback.
        // Keep their visible view updating through the frame that consumes completion.
        let refresh = self.share.guest_requests_busy()
            || (self.right_tool_visible(SideTab::Files) && self.explorer_tree.gpui_needs_frame())
            || ((self.side_panels_ui.sessions_modal_open
                || self.right_tool_visible(SideTab::History))
                && self.gpui_extra.session_query.pending())
            || (self.view == View::Remote && self.remote.gpui_needs_frame())
            || (self.right_tool_visible(SideTab::Conversations)
                && (self.gpui_extra.conversation_loader.needs_frame()
                    || self
                        .gpui_extra
                        .conversation_model
                        .needs_frame(crate::side_panels_model::now_unix_ms())))
            || self
                .sessions
                .iter()
                .any(|session| session.profile_migration.is_some() || session.restarting.is_some())
            || self.transcript_trash_busy
            || self.gpui_extra.disk_comparison_pending
            || self
                .gpui_extra
                .outline_cache
                .as_ref()
                .is_some_and(|c| c.loading())
            || self.gpui_extra.session_discovery.busy()
            || self.host_import.is_some()
            || !self.host_tests.is_empty()
            || !self.document_pending.is_empty()
            || (self.view == View::Files
                && self.documents.get(self.active_document).is_some_and(|doc| {
                    doc.pdf
                        .as_ref()
                        .is_some_and(crate::pdf::PdfView::is_rendering)
                        || doc
                            .office
                            .as_ref()
                            .is_some_and(crate::office::OfficeView::is_rendering)
                }));
        self.native_hosts.poll();
        let native_changed = self.poll();
        self.floating.poll();
        self.sync_live_share();
        self.flush_gpui_terminal_side_effects();
        refresh || native_changed
    }

    fn flush_gpui_terminal_side_effects(&mut self) {
        let mut pending_url: Option<String> = None;
        let mut broadcasts: Vec<(u64, Vec<crate::terminal::BroadcastInput>)> = Vec::new();
        for sess in &mut self.sessions {
            let agent_tag = match &sess.launch {
                Launch::Agent(a) => Some(a.clone()),
                _ => None,
            };
            if let Some(term) = &mut sess.terminal {
                term.set_agent_kind(agent_tag.as_deref());
                if let Some(url) = term.take_pending_tessel_url() {
                    pending_url = Some(url);
                }
                let out = term.take_broadcast_inputs();
                if !out.is_empty() {
                    broadcasts.push((sess.id, out));
                }
            }
        }
        if let Some(term) = self.floating.process_mut()
            && let Some(url) = term.take_pending_tessel_url()
        {
            pending_url = Some(url);
        }
        if self.state.broadcast && !broadcasts.is_empty() {
            let active_proj = self.state.active_project;
            for (src_id, items) in broadcasts {
                let src_in_broadcast = self
                    .sessions
                    .iter()
                    .any(|s| s.id == src_id && s.project == active_proj && s.broadcast);
                if !src_in_broadcast {
                    continue;
                }
                for sess in &mut self.sessions {
                    if sess.id != src_id
                        && sess.project == active_proj
                        && sess.broadcast
                        && let Some(term) = &mut sess.terminal
                    {
                        for item in &items {
                            term.send_broadcast(item);
                        }
                    }
                }
            }
        }
        if let Some(url) = pending_url {
            self.open_tessel_url_headless(url);
        }
    }

    fn open_tessel_url_headless(&mut self, url: String) {
        #[cfg(windows)]
        {
            self.browser_tab_open = true;
            self.browser.open_url(&url);
            self.view = View::Browser;
        }
        #[cfg(not(windows))]
        {
            let _ = url;
        }
    }

    /// Forwards a `GPUI` keystroke directly to the target session or floating terminal PTY.
    /// Returns `Some(copied_text)` when `Ctrl+C` (with active selection) or `Ctrl+Shift+C` copies text.
    pub(crate) fn send_gpui_terminal_key(
        &mut self,
        session_id: Option<u64>,
        key: &str,
        key_char: Option<&str>,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> Option<String> {
        let mut copied = None;
        self.with_target_terminal(session_id, |terminal| {
            copied = terminal.gpui_send_key(key, key_char, ctrl, alt, shift, session_id.is_none());
        });
        self.flush_gpui_terminal_side_effects();
        copied
    }

    pub(crate) fn gpui_take_copied_clipboard(&mut self) -> Option<String> {
        self.gpui_extra.copied_clipboard.take()
    }

    /// The toolbar's `⋯` menu (`App.vue` `.tb-more`), shared by both renderers.
    pub(crate) fn apply_toolbar_more_action(&mut self, action: crate::toolbar_fit::MoreMenuAction) {
        use crate::layout_broadcast_ui::LayoutMenuAction as L;
        use crate::toolbar_fit::MoreMenuAction as M;
        match action {
            M::LiveShare => self.overlay = Some(Overlay::Share),
            M::GitHub => self.open_github_modal(self.state.active_project),
            M::Linear => self.open_linear_modal(),
            M::Broadcast => {
                self.state.broadcast = !self.state.broadcast;
                self.save();
            }
            M::FloatingTerminal => self.toggle_floating_headless(),
            M::SplitRight => self.apply_layout_action(L::SplitRight),
            M::SplitDown => self.apply_layout_action(L::SplitDown),
            M::ArrangeGrid => self.apply_layout_action(L::ArrangeGrid),
            M::CloseActivePane => self.apply_layout_action(L::CloseActive),
            M::ToggleLeftSidebar => {
                self.state.panels.left_open = !self.state.panels.left_open;
                self.save();
            }
            M::ToggleRightSidebar => {
                self.state.panels.right_open = !self.state.panels.right_open;
                if !self.state.panels.right_open {
                    self.right_fullscreen = false;
                }
                self.save();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn gpui_is_palette_open(&self) -> bool {
        self.command_palette.open
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_sessions_modal_fixture(&mut self, history_visible: bool) {
        self.state.panels.right_open = history_visible;
        self.state.panels.right_tab = SideTab::History;
        self.side_panels_ui.sessions_modal_open = true;
        // Empty synthetic discovery must never fall back to real user profiles.
        self.gpui_extra.session_discovery.request(None, vec![], 50);
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_scm_pending_fixture(&mut self) -> mpsc::Sender<Result<String, String>> {
        self.sync_git_project();
        let owner = self.gpui_scm_owner().unwrap();
        let (sender, receiver) = mpsc::channel();
        assert!(self.git_result.start(owner.clone(), receiver));
        self.gpui_extra.scm_drafts.submitted(&owner);
        self.commit_in_flight = true;
        sender
    }
    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_scm_status_fixture(&mut self) {
        self.sync_git_project();
        let owner = self.gpui_scm_owner().unwrap();
        let capture = crate::git::scoped_status(&owner.root).unwrap();
        self.scm_state.result = Some(Ok(crate::scm_changes_panel::parse_porcelain_v2_status(
            &capture.output.bytes,
            crate::scm_changes_panel::STATUS_ENTRIES_MAX,
        )));
    }

    fn gpui_panel_query_context(
        &self,
    ) -> Option<(GpuiFocusTarget, Option<crate::git_result::Owner>, bool)> {
        if self.side_panels_ui.sessions_modal_open {
            if self.command_palette.open
                || self.finder.is_open()
                || self.overlay.is_some()
                || self.gpui_has_aux_modal_except_sessions()
            {
                return None;
            }
            return Some((GpuiFocusTarget::SessionsQuery, self.gpui_scm_owner(), true));
        }
        if self.gpui_has_open_modal() || !self.right_tool_visible(self.state.panels.right_tab) {
            return None;
        }
        let target = match self.state.panels.right_tab {
            SideTab::Files
                if self
                    .gpui_extra
                    .outline_query
                    .as_ref()
                    .is_some_and(|(id, _)| self.outline_document_id() == Some(*id)) =>
            {
                GpuiFocusTarget::OutlineQuery
            }
            SideTab::Tasks if self.gpui_scm_owner().is_some() => GpuiFocusTarget::TaskNewTitle,
            SideTab::Dashboard => GpuiFocusTarget::DashboardQuery,
            SideTab::TaskHistory => GpuiFocusTarget::TaskHistoryQuery,
            SideTab::History => GpuiFocusTarget::SessionsQuery,
            SideTab::Conversations => GpuiFocusTarget::ConversationsQuery,
            _ => return None,
        };
        Some((target, self.gpui_scm_owner(), false))
    }
    pub(crate) fn gpui_panel_query_input(&self) -> Option<(NativePanelQueryScope, &str)> {
        let target = self.gpui_panel_query_context()?.0;
        let text = match target {
            GpuiFocusTarget::OutlineQuery => &self.gpui_extra.outline_query.as_ref()?.1,
            GpuiFocusTarget::TaskNewTitle => &self.gpui_extra.new_task_title,
            GpuiFocusTarget::DashboardQuery => &self.side_panels_ui.dashboard_query,
            GpuiFocusTarget::TaskHistoryQuery => &self.side_panels_ui.history_query,
            GpuiFocusTarget::SessionsQuery => &self.side_panels_ui.sessions_query,
            GpuiFocusTarget::ConversationsQuery => &self.side_panels_ui.conv_query,
            _ => return None,
        };
        Some((
            NativePanelQueryScope {
                generation: self.gpui_extra.panel_query_generation,
                target,
            },
            text,
        ))
    }

    pub(crate) fn outline_document_id(&self) -> Option<u64> {
        if self.gpui_has_open_modal()
            || !self.gpui_extra.outline_open
            || !self.right_tool_visible(SideTab::Files)
        {
            return None;
        }
        self.documents
            .get(self.local_explorer_document_index()?)
            .filter(|doc| crate::json_outline_panel::supported(doc))
            .map(|doc| doc.instance_id())
    }

    fn gpui_explorer_owner(&self) -> Option<crate::git_result::Owner> {
        if self.gpui_has_open_modal() || !self.right_tool_visible(SideTab::Files) {
            return None;
        }
        self.gpui_scm_owner()
    }
    pub(crate) fn gpui_explorer_input(&self) -> Option<(NativeExplorerScope, &str)> {
        Some((
            NativeExplorerScope {
                generation: self.gpui_extra.explorer_input_generation,
                owner: self.gpui_explorer_owner()?,
            },
            self.explorer_tree.gpui_filter_query(),
        ))
    }

    pub(crate) fn gpui_scm_input(&self) -> Option<(NativeScmScope, &str)> {
        if self.gpui_has_open_modal() || !self.right_tool_visible(SideTab::Changes) {
            return None;
        }
        self.gpui_scm_draft()
            .map(|(owner, id, text)| (NativeScmScope { owner, id }, text))
    }
    pub(crate) fn gpui_scm_can_submit(&self) -> bool {
        !self.git_result.busy() && self.scm_state.validate_commit().is_ok()
    }
    pub(crate) fn gpui_scm_busy(&self) -> bool {
        self.gpui_scm_owner()
            .is_some_and(|owner| self.gpui_extra.scm_drafts.pending(&owner))
    }
    pub(crate) fn gpui_scm_input_ids(&self) -> Vec<u64> {
        self.gpui_extra.scm_drafts.ids()
    }

    pub(crate) fn gpui_scm_owner(&self) -> Option<crate::git_result::Owner> {
        self.state.active().map(|p| crate::git_result::Owner {
            project: p.id,
            root: p.path.clone(),
        })
    }
    pub(crate) fn gpui_scm_draft(&self) -> Option<(crate::git_result::Owner, u64, &str)> {
        let owner = self.gpui_scm_owner()?;
        let (id, text) = self.gpui_extra.scm_drafts.input(&owner)?;
        Some((owner, id, text))
    }

    pub(crate) fn gpui_modal_query(&self) -> Option<((u64, GpuiFocusTarget), &str)> {
        if self.gpui_has_aux_modal() {
            return None;
        }
        let (target, text) = match self.overlay {
            Some(Overlay::Help) => (
                GpuiFocusTarget::HelpQuery,
                self.gpui_extra.help_query.as_str(),
            ),
            Some(Overlay::Settings) => (
                GpuiFocusTarget::SettingsSearch,
                self.gpui_extra.settings_search.as_str(),
            ),
            Some(Overlay::Share) if self.share.tab == crate::share_panel::DialogTab::Join => {
                let (scope, text) = self.share.guest_task_input()?;
                (GpuiFocusTarget::GuestTaskDraft(scope), text)
            }
            _ => return None,
        };
        Some(((self.gpui_extra.modal_input_generation, target), text))
    }

    pub(crate) fn gpui_dismiss_inner_overlay(&mut self) -> bool {
        if self.overlay == Some(Overlay::Settings)
            && self.current_settings_page_id() == "ssh"
            && self.gpui_extra.editing_ssh.is_some()
        {
            self.apply_gpui_action(GpuiAction::CancelSshEdit);
            return true;
        }
        if self.overlay == Some(Overlay::Help) && !self.gpui_extra.help_query.is_empty() {
            self.gpui_extra.help_query.clear();
            return true;
        }
        false
    }

    pub(crate) fn gpui_native_search_input(&self) -> Option<(NativeSearchScope, &str)> {
        if self.gpui_has_non_search_modal() {
            return None;
        }
        let (kind, generation, text) = if let Some((generation, text)) = self.finder.native_input()
        {
            (NativeSearchKind::Finder, generation, text)
        } else if self.command_palette.open {
            (
                NativeSearchKind::Palette,
                self.command_palette.generation,
                self.command_palette.query.as_str(),
            )
        } else {
            return None;
        };
        Some((
            NativeSearchScope {
                kind,
                generation,
                project: self.state.active_project,
            },
            text,
        ))
    }

    pub(crate) fn hover_search_result(
        &mut self,
        scope: NativeSearchScope,
        query: &str,
        index: usize,
        choice: &str,
    ) -> bool {
        if self.gpui_native_search_input() != Some((scope, query)) {
            return false;
        }
        match scope.kind {
            NativeSearchKind::Finder => self.finder.hover_result(index, choice),
            NativeSearchKind::Palette => {
                if self.command_palette.index == index {
                    return false;
                }
                let cmds =
                    self.build_palette_commands(self.state.theme.unwrap_or(ThemeId::Classic));
                if crate::palette::filter_palette_commands(&cmds, query)
                    .get(index)
                    .is_none_or(|row| row.command.id != choice)
                {
                    return false;
                }
                self.command_palette.index = index;
                true
            }
        }
    }
    pub(crate) fn gpui_native_search_choice(&self, scope: NativeSearchScope) -> Option<String> {
        let (current, query) = self.gpui_native_search_input()?;
        if current != scope {
            return None;
        }
        match scope.kind {
            NativeSearchKind::Finder => self.finder.native_selected_path().map(str::to_owned),
            NativeSearchKind::Palette => {
                let commands =
                    self.build_palette_commands(self.state.theme.unwrap_or(ThemeId::Classic));
                crate::palette::filter_palette_commands(&commands, query)
                    .get(self.command_palette.index)
                    .map(|r| r.command.id.clone())
            }
        }
    }

    pub(crate) fn gpui_issues_menu_open(&self) -> bool {
        self.gpui_extra.issues_menu_open
    }
    #[cfg(windows)]
    pub(crate) fn gpui_media_unobscured(&self) -> bool {
        !self.gpui_has_open_modal() && !self.floating.ui.is_open()
    }

    #[cfg(windows)]
    fn active_html_folder(&self) -> Option<(u64, crate::browser::html_trust::Folder)> {
        use crate::browser::html_trust::Folder;
        match self.view {
            View::Files => {
                let doc = self.documents.get(self.active_document)?;
                (doc.preview_mode
                    && doc.tools.html_plain_text.is_none()
                    && !doc.media
                    && doc
                        .path
                        .extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|ext| {
                            ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")
                        }))
                .then(|| {
                    (
                        doc.instance_id(),
                        Folder::Local(
                            doc.tools
                                .html_root
                                .clone()
                                .unwrap_or_else(|| doc.root.clone()),
                        ),
                    )
                })
            }
            View::Remote => {
                let id = self.remote.html_page_id()?;
                let (host, root, _, _) = self.remote.active_html_page(id)?;
                Some((
                    id,
                    Folder::Ssh {
                        host: host.connection_key(),
                        root: root.into(),
                    },
                ))
            }
            _ => None,
        }
    }
    #[cfg(windows)]
    pub(crate) fn sync_html_page(
        &mut self,
        window: &impl wry::raw_window_handle::HasWindowHandle,
        id: u64,
        rect: eframe::egui::Rect,
        scale: f32,
    ) {
        let Some((active, folder)) = self.active_html_folder() else {
            return;
        };
        if active != id {
            return;
        }
        self.editor_page
            .set_scripts(self.html_trust.decision(&folder) == Some(true));
        if self.view == View::Remote
            && self.gpui_media_unobscured()
            && let Some((host, root, path, text)) = self.remote.active_html_page(id)
        {
            self.editor_page
                .sync_source(window, id, rect, scale, true, |pages| {
                    pages.open_remote(host, root, path, text)
                });
            return;
        }
        if self.view == View::Files
            && self.gpui_media_unobscured()
            && let Some(doc) = self.documents.get(self.active_document)
            && doc.instance_id() == id
            && doc.preview_mode
            && doc.tools.html_plain_text.is_none()
            && doc.tools.html_link_menu.is_none()
        {
            self.editor_page.sync(window, doc, rect, scale);
        }
    }
    #[cfg(windows)]
    pub(crate) fn poll_html_page(&mut self) -> bool {
        if self.editor_page.poll_external() {
            return true;
        }
        if self.view == View::Files
            && self.gpui_media_unobscured()
            && let Some(doc) = self.documents.get(self.active_document)
            && doc.preview_mode
            && doc.tools.html_plain_text.is_none()
        {
            if let Some(action) = self.editor_page.action(doc.instance_id()) {
                self.apply_gpui_action(action);
                return true;
            }
            return self
                .editor_page
                .poll_draft(doc, std::time::Instant::now())
                .unwrap_or_else(|error| {
                    self.error = error;
                    true
                });
        }
        false
    }

    #[cfg(windows)]
    fn download_browser_mut(&mut self, tab: u64) -> Option<&mut crate::browser::Browser> {
        if self.browser.tab_id == tab {
            Some(&mut self.browser)
        } else if self.selected_side_browser == Some(tab) {
            self.side_browsers.get_mut(&tab)
        } else {
            None
        }
    }

    #[cfg(windows)]
    pub(crate) fn side_browser_visible(&self, id: u64) -> bool {
        self.selected_side_browser == Some(id)
            && self.state.panels.right_open
            && !self.gpui_has_open_modal()
            && !self.floating.ui.is_open()
    }

    pub(crate) fn gpui_has_open_modal(&self) -> bool {
        self.command_palette.open || self.finder.is_open() || self.gpui_has_non_search_modal()
    }

    fn gpui_has_non_search_modal(&self) -> bool {
        self.overlay.is_some() || self.gpui_has_aux_modal()
    }
    fn gpui_has_aux_modal(&self) -> bool {
        self.side_panels_ui.sessions_modal_open || self.gpui_has_aux_modal_except_sessions()
    }
    fn gpui_has_aux_modal_except_sessions(&self) -> bool {
        self.new_task.open
            || self.linear.view.open
            || self.gpui_extra.layout_menu_right.is_some()
            || self.gpui_extra.issues_menu_open
            || self.gpui_extra.launch_menu_open
            || self.gpui_extra.toolbar_more_open
            || self.dialogs_toolbar_ui.any_modal_open()
            || self.side_panels_ui.notes_modal_open
            || self.side_panels_ui.review_modal_open
            || self.worktree_cleanup.is_some()
            || self.teams_ui.manager_project.is_some()
            || !self.teams_hub.approval_prompts.is_empty()
            || self
                .teams_hub
                .workers
                .iter()
                .any(|w| w.state == crate::teams_relay::WorkerState::Confirming)
            || self.share.has_pending_ask()
            || self.rename_project_modal.is_some()
            || self.rename_agent.is_some()
            || self.pending_close.is_some()
            || self.pending_close_session.is_some()
            || self.pending_remove_project.is_some()
            || self.pending_scm_discard.is_some()
            || self.pending_delete_worktree.is_some()
            || self.pending_agent_install.is_some()
            || self.gpui_extra.task_board.pending_delete.is_some()
            || self.pending_transcript_trash.is_some()
            || self.file_action.is_some()
            || self.dev_update_dialog_open
    }

    pub(crate) fn gpui_guest_editor(&self) -> Option<crate::guest_file_editor::Document> {
        if self.view != View::GuestFiles || self.gpui_has_open_modal() || self.floating.ui.is_open()
        {
            return None;
        }
        self.share.editable_workspace_document()
    }

    pub(crate) fn gpui_edit_guest_document(
        &mut self,
        base: &crate::guest_file_editor::Document,
        text: String,
    ) -> bool {
        self.gpui_guest_editor()
            .is_some_and(|doc| doc.key == base.key && std::sync::Arc::ptr_eq(&doc.text, &base.text))
            && self.share.edit_guest_file(&base.key, text)
    }

    pub(crate) fn gpui_grid_receives_navigation(&self) -> bool {
        self.view == View::Work && !self.gpui_has_open_modal() && !self.floating.ui.is_open()
    }

    pub(crate) fn gpui_neighbor(&self, dir: FocusDir) -> Option<u64> {
        if !self.gpui_grid_receives_navigation() {
            return None;
        }
        let layout = self
            .state
            .terminal_layouts
            .get(&crate::terminal_layout::project_key(
                self.state.active_project,
            ))?;
        let visible = self.grid_worktree_ids();
        if layout
            .maximized
            .is_some_and(|id| visible.as_ref().is_none_or(|ids| ids.contains(&id)))
        {
            return None;
        }
        let panes = layout
            .view_geometry(visible.as_ref())
            .tiles
            .into_iter()
            .map(|tile| {
                (
                    tile.leaf.id,
                    tile.rect.to_px(self.grid_room[0], self.grid_room[1], 4.),
                )
            })
            .collect::<Vec<_>>();
        crate::pane_grid::move_focus(&panes, &self.active_session?.to_string(), dir)?
            .parse()
            .ok()
    }

    pub(crate) fn gpui_restore_maximized_pane(&mut self) -> bool {
        if !self.gpui_grid_receives_navigation() {
            return false;
        }
        let key = crate::terminal_layout::project_key(self.state.active_project);
        let Some(layout) = self.state.terminal_layouts.get_mut(&key) else {
            return false;
        };
        #[cfg(windows)]
        if self.sessions.iter().any(|s| {
            Some(s.id) == layout.maximized
                && s.terminal
                    .as_ref()
                    .is_some_and(|terminal| terminal.native_owns_escape())
        }) {
            return false;
        }
        layout.maximized.take().is_some()
    }

    pub(crate) fn gpui_can_split_shortcut(&self) -> bool {
        !self.gpui_has_open_modal()
            && !self.floating.ui.is_open()
            && !matches!(self.view, View::GuestFiles | View::Remote)
    }

    pub(crate) fn gpui_close_document_action(&self) -> Option<GpuiAction> {
        if self.gpui_has_open_modal() || self.floating.ui.is_open() {
            return None;
        }
        match self.view {
            View::Files if self.project_document(self.active_document) => {
                Some(GpuiAction::CloseDocumentTab(self.active_document))
            }
            View::GuestFiles => self.share.active_workspace_file().cloned().map(|key| {
                GpuiAction::Share(crate::share_panel::ShareGpuiAction::GuestCloseTab(key))
            }),
            _ => None,
        }
    }

    pub(crate) fn gpui_is_document_view(&self) -> bool {
        self.view == View::Files && self.project_document(self.active_document)
    }

    pub(crate) fn gpui_remote_input_snapshot(
        &mut self,
    ) -> Option<crate::remote_panel::control::Snapshot> {
        (self.view == View::Remote && !self.gpui_has_open_modal())
            .then(|| self.remote.input_snapshot())
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_terminal_find_fixture(
        &mut self,
    ) -> (u64, std::sync::mpsc::Receiver<Vec<u8>>) {
        self.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        let id = self.active_session.unwrap();
        let receiver = self.replace_terminal_find_fixture(id);
        self.claude_chats.retain(|c| c.pane != id);
        self.chats.retain(|c| c.pane != id);
        (id, receiver)
    }
    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn replace_terminal_find_fixture(
        &mut self,
        id: u64,
    ) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (terminal, receiver) = crate::terminal::Terminal::native_find_fixture();
        let session = self.sessions.iter_mut().find(|s| s.id == id).unwrap();
        // Shell fixtures must satisfy the same workspace validation as actual shell panes.
        session.profile_binding = None;
        session.session_options = None;
        session.launch = Launch::Shell;
        session.terminal = Some(terminal);
        receiver
    }
    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_floating_find_fixture(&mut self) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (terminal, receiver) = crate::terminal::Terminal::native_find_fixture();
        self.floating.install_find_fixture(terminal);
        receiver
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_finder_input_fixture(&mut self, root: std::path::PathBuf) {
        self.finder = crate::finder_panel::Finder::fixture(root);
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn complete_finder_scan_for_test(&mut self, files: Vec<String>) {
        self.finder.complete_scan_for_test(files);
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_remote_image_fixture(&mut self) {
        self.view = View::Remote;
        self.remote = crate::remote_panel::RemotePanel::fixture();
    }
    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_remote_input_fixture(&mut self) {
        self.view = View::Remote;
        self.remote = crate::remote_panel::RemotePanel::text_fixture();
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_remote_html_fixture(&mut self) {
        self.view = View::Remote;
        self.remote =
            crate::remote_panel::RemotePanel::text_fixture_with("index.HTM", "<p>Original</p>");
    }
    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn install_editor_input_fixture(
        &mut self,
        documents: Vec<crate::files::Document>,
        active: usize,
    ) {
        self.view = View::Files;
        self.documents = documents;
        for doc in &mut self.documents {
            doc.project = self.state.active_project;
        }
        self.active_document = active;
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn set_floating_input_fixture(&mut self, open: bool) {
        // Exercise focus routing without starting a shell on the shared workstation.
        if open {
            self.floating.ui.show();
        } else {
            self.floating.ui.hide();
        }
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn hold_remote_input_fixture(&mut self) -> impl FnOnce() + use<> {
        self.remote.hold_input_fixture()
    }

    #[cfg(all(test, feature = "visual-test"))]
    pub(crate) fn inject_codex_frame_for_test(&mut self, pane: u64, frame: serde_json::Value) {
        self.ensure_native_chat_instances();
        self.chats
            .iter_mut()
            .find(|c| c.pane == pane)
            .unwrap()
            .process_protocol_frame_for_test(frame);
    }

    #[cfg(test)]
    pub(crate) fn inject_claude_frame_for_test(&mut self, pane_id: u64, frame: serde_json::Value) {
        self.ensure_native_chat_instances();
        if let Some(chat) = self.claude_chats.iter_mut().find(|c| c.pane == pane_id) {
            chat.process_protocol_frame_for_test(frame);
        }
    }

    #[cfg(test)]
    pub(crate) fn gpui_chat_sender_for_test(
        &mut self,
        pane: u64,
    ) -> std::sync::mpsc::Receiver<serde_json::Value> {
        if let Some(c) = self.chats.iter_mut().find(|c| c.pane == pane) {
            c.ready_sender_for_test()
        } else {
            self.claude_chats
                .iter_mut()
                .find(|c| c.pane == pane)
                .unwrap()
                .ready_sender_for_test()
        }
    }

    // Input-only lookup: never clone transcripts on a keystroke. Validate the current
    // session provider, since old backend objects may outlive a renderer switch.
    pub(crate) fn gpui_chat_composer(&self, pane_id: u64) -> Option<(u64, &str)> {
        let session = self.sessions.iter().find(|s| s.id == pane_id)?;
        match session.launch {
            Launch::CodexChat => self
                .chats
                .iter()
                .find(|c| c.pane == pane_id)
                .map(|c| c.composer_state()),
            Launch::ClaudeChat => self
                .claude_chats
                .iter()
                .find(|c| c.pane == pane_id)
                .map(|c| c.composer_state()),
            _ => None,
        }
    }

    pub(crate) fn gpui_chat_can_interrupt(&self, pane: u64) -> bool {
        if !self.gpui_chat_input_allowed(pane) {
            return false;
        }
        match self
            .sessions
            .iter()
            .find(|s| s.id == pane)
            .map(|s| &s.launch)
        {
            Some(Launch::CodexChat) => self
                .chats
                .iter()
                .any(|c| c.pane == pane && c.can_interrupt_from_composer()),
            Some(Launch::ClaudeChat) => self
                .claude_chats
                .iter()
                .any(|c| c.pane == pane && c.can_interrupt_from_composer()),
            _ => false,
        }
    }

    pub(crate) fn gpui_chat_request_id(&self, pane: u64, approval: bool) -> Option<u64> {
        let session = self.sessions.iter().find(|s| s.id == pane)?;
        match session.launch {
            Launch::CodexChat => {
                let c = self.chats.iter().find(|c| c.pane == pane)?;
                if approval {
                    c.approval_instance_id()
                } else {
                    c.question_instance_id()
                }
            }
            Launch::ClaudeChat => {
                let c = self.claude_chats.iter().find(|c| c.pane == pane)?;
                if approval {
                    c.approval_instance_id()
                } else {
                    c.question_instance_id()
                }
            }
            _ => None,
        }
    }

    pub(crate) fn gpui_chat_question_state(&self, pane: u64, index: usize) -> Option<(u64, &str)> {
        let session = self.sessions.iter().find(|s| s.id == pane)?;
        match session.launch {
            Launch::CodexChat => self
                .chats
                .iter()
                .find(|c| c.pane == pane)?
                .question_state(index),
            Launch::ClaudeChat => self
                .claude_chats
                .iter()
                .find(|c| c.pane == pane)?
                .question_state(index),
            _ => None,
        }
    }

    pub(crate) fn gpui_chat_input_allowed(&self, pane_id: u64) -> bool {
        self.view == View::Work
            && !self.gpui_has_open_modal()
            && self
                .sessions
                .iter()
                .any(|s| s.id == pane_id && s.project == self.state.active_project)
    }

    pub(crate) fn gpui_chat_question_ids(&self) -> Vec<(u64, u64)> {
        self.sessions
            .iter()
            .filter_map(|s| self.gpui_chat_request_id(s.id, false).map(|id| (s.id, id)))
            .collect()
    }

    pub(crate) fn gpui_chat_composer_ids(&self) -> Vec<(u64, u64)> {
        self.sessions
            .iter()
            .filter_map(|s| self.gpui_chat_composer(s.id).map(|(id, _)| (s.id, id)))
            .collect()
    }

    pub(crate) fn gpui_browser_address(&self) -> Option<((u64, Option<u64>), &str)> {
        self.browser_address_for(false)
    }

    pub(crate) fn browser_address_for(&self, side: bool) -> Option<((u64, Option<u64>), &str)> {
        #[cfg(windows)]
        if side {
            let id = self.selected_side_browser?;
            if !self.side_browser_visible(id) {
                return None;
            }
            let page = self.side_browsers.get(&id)?;
            return Some((
                (page.address_generation(), Some(id)),
                page.display_address(),
            ));
        }
        #[cfg(not(windows))]
        let _ = side;
        #[cfg(windows)]
        if self.view == View::Browser
            && !self.right_fullscreen
            && !self.gpui_has_open_modal()
            && !self.floating.ui.is_open()
        {
            return Some((
                (self.browser.address_generation(), self.state.active_project),
                self.browser.display_address(),
            ));
        }
        None
    }

    #[cfg(windows)]
    fn address_page_mut(
        &mut self,
        scope: (u64, Option<u64>),
    ) -> Option<&mut crate::browser::Browser> {
        if self
            .browser_address_for(true)
            .is_some_and(|(active, _)| active == scope)
        {
            return self.side_browsers.get_mut(&scope.1?);
        }
        (self.gpui_browser_address()?.0 == scope).then_some(&mut self.browser)
    }

    #[cfg(windows)]
    pub(crate) fn take_new_address_focus(&mut self, scope: (u64, Option<u64>)) -> bool {
        if let Some(page) = self.address_page_mut(scope)
            && std::mem::take(&mut page.focus_new_address)
        {
            page.begin_native_address_edit();
            true
        } else {
            false
        }
    }

    pub(crate) fn gpui_terminal_find_ids(&self) -> Vec<u64> {
        self.sessions
            .iter()
            .filter_map(|s| s.terminal.as_ref()?.native_find_input().map(|v| v.0))
            .chain(
                self.floating
                    .process_ref()
                    .and_then(|t| t.native_find_input())
                    .map(|v| v.0),
            )
            .collect()
    }

    pub(crate) fn gpui_terminal_find_input(&self, sid: Option<u64>) -> Option<(u64, &str)> {
        if self.gpui_has_open_modal() {
            return None;
        }
        let terminal = match sid {
            Some(id) if self.view == View::Work && !self.floating.ui.is_open() => self
                .sessions
                .iter()
                .find(|s| s.id == id && s.project == self.state.active_project)?
                .terminal
                .as_ref()?,
            None if self.floating.ui.is_open() => self.floating.process_ref()?,
            _ => return None,
        };
        terminal.native_find_input()
    }

    pub(crate) fn gpui_editor_tool_document_id(&self) -> Option<u64> {
        if self.view != View::Files || self.gpui_has_open_modal() {
            return None;
        }
        self.documents
            .get(self.active_document)
            .map(crate::files::Document::instance_id)
    }

    pub(crate) fn gpui_editor_tool_value(&self, id: u64, field: EditorToolField) -> Option<&str> {
        if self.view != View::Files || self.gpui_has_open_modal() {
            return None;
        }
        let doc = self.documents.get(self.active_document)?;
        if doc.instance_id() != id {
            return None;
        }
        match field {
            EditorToolField::Find if doc.tools.find_open => Some(&doc.tools.find_query),
            EditorToolField::Replace
                if doc.tools.find_open
                    && doc.tools.replace_open
                    && !doc.media
                    && !doc.preview_mode =>
            {
                Some(&doc.tools.replace_text)
            }
            EditorToolField::Goto if doc.tools.goto_open => Some(&doc.tools.goto_line),
            _ => None,
        }
    }

    pub(crate) fn set_local_pdf_viewport(&mut self, id: u64, viewport: eframe::egui::Rect) -> bool {
        self.view == View::Files
            && self
                .documents
                .get_mut(self.active_document)
                .filter(|doc| doc.instance_id() == id)
                .and_then(|doc| doc.pdf.as_mut())
                .is_some_and(|pdf| pdf.set_native_viewport(viewport))
    }

    pub(crate) fn set_guest_pdf_viewport(&mut self, id: u64, viewport: eframe::egui::Rect) -> bool {
        self.share
            .guest_pdf_mut(id)
            .is_some_and(|pdf| pdf.set_native_viewport(viewport))
    }
    pub(crate) fn set_remote_pdf_viewport(
        &mut self,
        id: u64,
        viewport: eframe::egui::Rect,
    ) -> bool {
        self.view == View::Remote && self.remote.set_pdf_viewport(id, viewport)
    }

    pub(crate) fn navigate_editor_find(
        &mut self,
        id: u64,
        range: std::ops::Range<usize>,
        backwards: bool,
        query: Option<&str>,
    ) {
        if self.gpui_editor_tool_document_id() == Some(id)
            && let Some(doc) = self.documents.get_mut(self.active_document)
        {
            if let Some(query) = query
                && query != doc.tools.find_query
            {
                doc.tools.find_query.clear();
                doc.tools.find_query.push_str(query);
            }
            doc.navigate_find(range, backwards);
        }
    }

    pub(crate) fn seed_editor_find(
        &mut self,
        id: u64,
        selected: std::ops::Range<usize>,
        allow_word: bool,
    ) -> bool {
        if self.gpui_editor_tool_document_id() == Some(id)
            && let Some(doc) = self.documents.get_mut(self.active_document)
        {
            return doc.seed_find_from_selection(selected, allow_word);
        }
        false
    }

    pub(crate) fn gpui_editor_document(&self) -> Option<&crate::files::Document> {
        (self.view == View::Files && !self.gpui_has_open_modal())
            .then(|| self.documents.get(self.active_document))
            .flatten()
            .filter(|doc| !doc.preview_mode)
    }

    pub(crate) fn gpui_document_ids(&self) -> Vec<u64> {
        self.documents
            .iter()
            .map(crate::files::Document::instance_id)
            .collect()
    }

    pub(crate) fn record_outline_cursor(&mut self, id: u64, offset: usize) -> bool {
        let Some(doc) = self.documents.get_mut(self.active_document) else {
            return false;
        };
        if self.view != View::Files
            || doc.instance_id() != id
            || doc.media
            || doc.diff_title.is_some()
        {
            return false;
        }
        let cursor = Some(crate::files::OutlineCursor::Bytes(offset));
        if doc.tools.outline_cursor == cursor {
            return false;
        }
        doc.tools.outline_cursor = cursor;
        !doc.preview_mode && crate::json_outline_panel::supported(doc)
    }

    pub(crate) fn take_outline_focus(&mut self, id: u64) -> Option<bool> {
        let doc = self.documents.get_mut(self.active_document)?;
        if self.view != View::Files || doc.instance_id() != id {
            return None;
        }
        doc.tools.outline_reveal_focus.take()
    }

    pub(crate) fn gpui_take_editor_selection(
        &mut self,
        instance_id: u64,
    ) -> Option<(usize, usize)> {
        if self.gpui_editor_document()?.instance_id() != instance_id {
            return None;
        }
        let doc = self.documents.get_mut(self.active_document)?;
        if let Some((start, end)) = doc.tools.pending_char_range.take() {
            doc.tools.pending_scroll_line = None;
            return Some(crate::files::char_range_to_byte_range(
                &doc.text, start, end,
            ));
        }
        let line = doc.tools.pending_scroll_line.take()?;
        let offset = doc
            .text
            .split_inclusive('\n')
            .take(line)
            .map(str::len)
            .sum::<usize>();
        Some((offset, offset))
    }

    pub(crate) fn gpui_task_agent_focus(
        &self,
        scope: crate::native_task_board::Scope,
        id: u64,
        pane: u64,
    ) -> Option<GpuiFocusTarget> {
        (self.gpui_extra.task_board.scope == Some(scope)
            && self.active_session == Some(pane)
            && crate::task_agents_model::can_focus(&self.state, scope.project, id, pane))
        .then(|| {
            if self
                .sessions
                .iter()
                .find(|s| s.id == pane)
                .is_some_and(|s| s.launch.native_chat().is_some())
            {
                GpuiFocusTarget::ChatComposer(pane)
            } else {
                GpuiFocusTarget::Terminal(Some(pane))
            }
        })
    }
    pub(crate) fn gpui_default_focus_target(&self) -> GpuiFocusTarget {
        if self.floating.ui.is_open() {
            return GpuiFocusTarget::Terminal(None);
        }
        match self.view {
            View::Work => {
                if let Some(sid) = self.active_session
                    && let Some(sess) = self.sessions.iter().find(|s| s.id == sid)
                {
                    if sess.launch.native_chat().is_some() {
                        GpuiFocusTarget::ChatComposer(sid)
                    } else {
                        GpuiFocusTarget::Terminal(Some(sid))
                    }
                } else {
                    GpuiFocusTarget::None
                }
            }
            View::Files => GpuiFocusTarget::EditorBody,
            View::GuestFiles | View::Remote => GpuiFocusTarget::None,
            #[cfg(windows)]
            View::Browser | View::EmptyBrowserGrid => GpuiFocusTarget::None,
        }
    }

    pub(crate) fn gpui_handle_text_target_key(
        &mut self,
        target: GpuiFocusTarget,
        key: &str,
        printable: Option<&str>,
        shift: bool,
        pasted: Option<String>,
    ) -> bool {
        self.ensure_native_chat_instances();
        let lower = key.to_ascii_lowercase();
        match target {
            GpuiFocusTarget::RemoteName
            | GpuiFocusTarget::RemoteRename
            | GpuiFocusTarget::RemoteText => {
                use crate::remote_panel::control::Command;
                if self.view != View::Remote || self.remote.busy() {
                    return false;
                }
                let snapshot = self.remote.snapshot(&self.state.hosts);
                let mut text = match target {
                    GpuiFocusTarget::RemoteName => snapshot.new_name,
                    GpuiFocusTarget::RemoteRename => {
                        snapshot.rename.map(|(_, name)| name).unwrap_or_default()
                    }
                    _ => snapshot
                        .document
                        .and_then(|(_, text, _)| text)
                        .unwrap_or_default(),
                };
                if lower == "enter" && target == GpuiFocusTarget::RemoteRename {
                    self.remote
                        .command(Command::ConfirmRename, &self.state.hosts);
                    return true;
                }
                if let Some(paste) = pasted {
                    text.push_str(&paste);
                } else if lower == "backspace" {
                    text.pop();
                } else if lower == "enter" && target == GpuiFocusTarget::RemoteText {
                    text.push('\n');
                } else if let Some(printable) = printable {
                    text.push_str(printable);
                } else {
                    return false;
                }
                self.remote.command(
                    match target {
                        GpuiFocusTarget::RemoteName => Command::Name(text),
                        GpuiFocusTarget::RemoteRename => Command::RenameDraft(text),
                        _ => Command::Edit(text),
                    },
                    &self.state.hosts,
                );
                true
            }
            GpuiFocusTarget::None | GpuiFocusTarget::Terminal(_) => false,
            GpuiFocusTarget::TerminalFind(_) => false,
            GpuiFocusTarget::ChatComposer(_) => false,
            GpuiFocusTarget::ChatQuestion { .. } => false,
            GpuiFocusTarget::EditorFind
            | GpuiFocusTarget::EditorReplace
            | GpuiFocusTarget::EditorGoto => false,
            // Only the native entity may edit document text; root focus must never append.
            GpuiFocusTarget::EditorBody => false,
            GpuiFocusTarget::BrowserUrl => false,
            GpuiFocusTarget::PaletteQuery | GpuiFocusTarget::FinderQuery => false,
            GpuiFocusTarget::HelpQuery => false,
            GpuiFocusTarget::ExplorerFilter
            | GpuiFocusTarget::ExplorerTree
            | GpuiFocusTarget::OutlineTree
            | GpuiFocusTarget::OutlineQuery => false,
            GpuiFocusTarget::ScmCommitMessage => false,
            GpuiFocusTarget::TaskNewTitle
            | GpuiFocusTarget::TaskRenameTitle
            | GpuiFocusTarget::TaskDecisionAnswer
            | GpuiFocusTarget::TaskCard(_)
            | GpuiFocusTarget::TaskBoardControls
            | GpuiFocusTarget::TaskBoardConfirm => false,
            GpuiFocusTarget::DashboardQuery
            | GpuiFocusTarget::TaskHistoryQuery
            | GpuiFocusTarget::SessionsQuery
            | GpuiFocusTarget::ConversationsQuery => false,
            GpuiFocusTarget::SettingsSearch => false,
            GpuiFocusTarget::NotesEditor => {
                let mut cur = self.side_panels_ui.notes.text.clone();
                if lower == "enter" {
                    cur.push('\n');
                    self.apply_gpui_action(GpuiAction::SetNotesText(cur));
                    true
                } else if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::SetNotesText(cur));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::AddProjectCloneUrl => {
                let mut cur = self.dialogs_toolbar_ui.clone_url.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::SetAddProjectCloneUrl(cur));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::AddProjectFolderName => {
                let mut cur = match self.dialogs_toolbar_ui.add_tab {
                    crate::dialogs_toolbar_ui::AddProjectTab::CloneRepo => {
                        self.dialogs_toolbar_ui.clone_folder_name.clone()
                    }
                    crate::dialogs_toolbar_ui::AddProjectTab::SshRemote => {
                        self.dialogs_toolbar_ui.ssh_remote_dir.clone()
                    }
                    _ => self.dialogs_toolbar_ui.init_folder_name.clone(),
                };
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::SetAddProjectFolderName(cur));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::GitHubSearch => {
                let mut cur = self.dialogs_toolbar_ui.github_query.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::SetGitHubQuery(cur));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::GuestTaskDraft(_) => false,
            GpuiFocusTarget::LiveShareJoinCode => {
                if lower == "enter" {
                    self.apply_gpui_action(GpuiAction::Share(
                        crate::share_panel::ShareGpuiAction::JoinSession,
                    ));
                    return true;
                }
                let mut cur = self.share.gpui_snapshot(false, false).join_code;
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::Share(
                        crate::share_panel::ShareGpuiAction::SetJoinCode(cur),
                    ));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::PromptDraft => {
                if lower == "enter" {
                    self.apply_gpui_action(GpuiAction::SubmitPromptModal);
                    return true;
                }
                let mut cur = self.current_prompt_draft();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::SetPromptDraft(cur));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::NewTeamName => {
                let mut cur = self.teams_ui.new_team_name.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::SetNewTeamName(cur));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::QuickCommandName => {
                let mut cur = self.gpui_extra.new_quick_cmd_name.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    let text = self.gpui_extra.new_quick_cmd_text.clone();
                    let press_enter = self.gpui_extra.new_quick_cmd_enter;
                    self.apply_gpui_action(GpuiAction::SetQuickCommandDraft {
                        name: cur,
                        text,
                        press_enter,
                    });
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::QuickCommandText => {
                if lower == "enter" && !shift {
                    self.apply_gpui_action(GpuiAction::SubmitNewQuickCommand);
                    return true;
                }
                let mut cur = self.gpui_extra.new_quick_cmd_text.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    let name = self.gpui_extra.new_quick_cmd_name.clone();
                    let press_enter = self.gpui_extra.new_quick_cmd_enter;
                    self.apply_gpui_action(GpuiAction::SetQuickCommandDraft {
                        name,
                        text: cur,
                        press_enter,
                    });
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::CustomAgentName => {
                let mut cur = self.gpui_extra.new_custom_agent_name.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    let cmd = self.gpui_extra.new_custom_agent_cmd.clone();
                    self.apply_gpui_action(GpuiAction::SetCustomAgentDraft { name: cur, cmd });
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::CustomAgentCmd => {
                if lower == "enter" {
                    self.apply_gpui_action(GpuiAction::SubmitNewCustomAgent);
                    return true;
                }
                let mut cur = self.gpui_extra.new_custom_agent_cmd.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    let name = self.gpui_extra.new_custom_agent_name.clone();
                    self.apply_gpui_action(GpuiAction::SetCustomAgentDraft { name, cmd: cur });
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::AccountLabel => {
                let mut cur = self.gpui_extra.new_account_label.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::SetAccountLabelDraft(cur));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::ProviderSecretValue => {
                if lower == "enter" {
                    self.apply_gpui_action(GpuiAction::SaveSecretFromDraft);
                    return true;
                }
                let mut cur = self.gpui_extra.secret_value_draft.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::SetSecretValueDraft(cur));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::SkillsGitUrl => {
                let mut cur = self.gpui_extra.skills_git_url.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    self.apply_gpui_action(GpuiAction::SetSkillsGitUrlDraft(cur));
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::SshAlias => {
                if lower == "enter" {
                    self.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
                    return true;
                }
                let mut cur = self.gpui_extra.new_ssh_alias.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    let folder = self.gpui_extra.new_ssh_folder.clone();
                    self.apply_gpui_action(GpuiAction::SetSshDraft { alias: cur, folder });
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::SshFolder => {
                if lower == "enter" {
                    self.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
                    return true;
                }
                let mut cur = self.gpui_extra.new_ssh_folder.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    let alias = self.gpui_extra.new_ssh_alias.clone();
                    self.apply_gpui_action(GpuiAction::SetSshDraft { alias, folder: cur });
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::AutomationName => {
                let mut cur = self.gpui_extra.new_auto_name.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    let prompt = self.gpui_extra.new_auto_prompt.clone();
                    let every_mins = self.gpui_extra.new_auto_every_mins;
                    self.apply_gpui_action(GpuiAction::SetAutomationDraft {
                        name: cur,
                        prompt,
                        every_mins,
                    });
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::AutomationPrompt => {
                if lower == "enter" && !shift {
                    self.apply_gpui_action(GpuiAction::SubmitNewAutomation);
                    return true;
                }
                let mut cur = self.gpui_extra.new_auto_prompt.clone();
                if apply_singleline_edit(&mut cur, &lower, printable, pasted) {
                    let name = self.gpui_extra.new_auto_name.clone();
                    let every_mins = self.gpui_extra.new_auto_every_mins;
                    self.apply_gpui_action(GpuiAction::SetAutomationDraft {
                        name,
                        prompt: cur,
                        every_mins,
                    });
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::BuiltinAgentCommand => {
                let Some(aid) = self.gpui_extra.expanded_agent_id.clone() else {
                    return false;
                };
                let mut prefs = crate::agent_preferences::get(&self.state, &aid)
                    .cloned()
                    .unwrap_or_default();
                if apply_singleline_edit(&mut prefs.command, &lower, printable, pasted) {
                    self.save_builtin_agent_prefs(&aid, prefs);
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::BuiltinAgentArgs => {
                let Some(aid) = self.gpui_extra.expanded_agent_id.clone() else {
                    return false;
                };
                let mut prefs = crate::agent_preferences::get(&self.state, &aid)
                    .cloned()
                    .unwrap_or_default();
                if apply_singleline_edit(&mut prefs.args, &lower, printable, pasted) {
                    self.save_builtin_agent_prefs(&aid, prefs);
                    true
                } else {
                    false
                }
            }
            GpuiFocusTarget::BuiltinAgentEnv => {
                let Some(aid) = self.gpui_extra.expanded_agent_id.clone() else {
                    return false;
                };
                let mut prefs = crate::agent_preferences::get(&self.state, &aid)
                    .cloned()
                    .unwrap_or_default();
                if lower == "enter" {
                    prefs.env.push('\n');
                    self.save_builtin_agent_prefs(&aid, prefs);
                    true
                } else if apply_singleline_edit(&mut prefs.env, &lower, printable, pasted) {
                    self.save_builtin_agent_prefs(&aid, prefs);
                    true
                } else {
                    false
                }
            }
        }
    }

    fn current_prompt_draft(&self) -> String {
        if let Some((_, draft, _)) = &self.rename_project_modal {
            draft.clone()
        } else if let Some(ra) = &self.rename_agent {
            ra.draft.clone()
        } else if self.dialogs_toolbar_ui.group_modal_project.is_some() {
            self.dialogs_toolbar_ui.group_modal_draft.clone()
        } else if let Some(fa) = &self.file_action {
            fa.name.clone()
        } else {
            String::new()
        }
    }

    #[cfg(test)]
    pub(crate) fn synthetic_wait_for_terminal_operation(&mut self, pane: u64) {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while self
            .sessions
            .iter()
            .any(|s| s.id == pane && (s.loading.is_some() || s.restarting.is_some()))
        {
            self.poll_for_gpui();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(test)]
    pub(crate) fn synthetic_native_profiles(&mut self, root: &std::path::Path) {
        for agent in ["claude", "codex"] {
            self.state.agent_prefs.insert(
                agent.into(),
                crate::agent_preferences::Preferences {
                    command: "tessel-synthetic-cli-does-not-exist".into(),
                    env: format!(
                        "{}={}",
                        crate::session_profile::env_key(agent).unwrap(),
                        root.join(agent).display()
                    ),
                    ..Default::default()
                },
            );
        }
    }

    pub(super) fn ensure_native_chat_instances(&mut self) {
        let panes: Vec<_> = self
            .sessions
            .iter()
            .filter_map(|s| {
                let provider = s.launch.native_chat()?;
                let pid = s.project?;
                let saved = self
                    .state
                    .native_resume(s.id, pid, provider)
                    .map(str::to_owned)
                    .or_else(|| s.conversation_id.clone());
                Some((
                    s.id,
                    pid,
                    provider,
                    s.cwd.clone(),
                    saved,
                    s.session_options.clone(),
                ))
            })
            .collect();
        let mut changed = false;
        for (pane, pid, provider, cwd, saved, options) in panes {
            let agent = provider.to_ascii_lowercase();
            if provider == "Codex" && !self.chats.iter().any(|c| c.pane == pane) {
                let mut chat = crate::chat::Chat::new(pid, cwd.clone());
                chat.pane = pane;
                chat.require_profile = true;
                if let Some(id) = &saved {
                    chat.set_resume(id.clone());
                }
                chat.options = crate::native_chat_options::Options::from_value(options.as_ref());
                self.chats.push(chat);
            } else if provider == "Claude" && !self.claude_chats.iter().any(|c| c.pane == pane) {
                let mut chat = crate::claude::Chat::new(pid, cwd, saved.clone());
                chat.pane = pane;
                chat.require_profile = true;
                chat.options = crate::native_chat_options::Options::from_value(options.as_ref());
                self.claude_chats.push(chat);
            }
            let (needs_profile, retry) = if provider == "Codex" {
                let chat = self
                    .chats
                    .iter_mut()
                    .find(|c| c.pane == pane)
                    .expect("native instance");
                (
                    chat.require_profile && chat.profile.is_none(),
                    std::mem::take(&mut chat.retry_profile),
                )
            } else {
                let chat = self
                    .claude_chats
                    .iter_mut()
                    .find(|c| c.pane == pane)
                    .expect("native instance");
                (
                    chat.require_profile && chat.profile.is_none(),
                    std::mem::take(&mut chat.retry_profile),
                )
            };
            if !needs_profile {
                continue;
            }
            if retry && let Some(session) = self.sessions.iter_mut().find(|s| s.id == pane) {
                session.error.clear();
            }
            let resolution = self.prepare_native_profile(pane, &agent, saved.as_deref());
            let (profile, error, status) = match resolution {
                Ok(Some(profile)) => {
                    changed = true;
                    (Some(profile), String::new(), "Not connected")
                }
                Ok(None) => (None, String::new(), "Identifying original account…"),
                Err(error) => (None, error, "Account unavailable"),
            };
            if provider == "Codex" {
                let chat = self
                    .chats
                    .iter_mut()
                    .find(|c| c.pane == pane)
                    .expect("native instance");
                chat.profile = profile;
                chat.error = error;
                chat.status = status.into();
            } else {
                let chat = self
                    .claude_chats
                    .iter_mut()
                    .find(|c| c.pane == pane)
                    .expect("native instance");
                chat.profile = profile;
                chat.error = error;
                chat.status = status.into();
            }
        }
        if changed {
            self.save();
        }
    }

    pub(crate) fn gpui_snapshot(
        &mut self,
        grid_room_w: f32,
        grid_room_h: f32,
        divider_px: f32,
    ) -> GpuiWorkspaceSnapshot {
        self.reconcile_native_task_board();
        use crate::status_metrics::should_keep_awake;

        self.ensure_native_chat_instances();

        let room = [
            f64::from(grid_room_w.max(320.0)),
            f64::from(grid_room_h.max(180.0)),
        ];
        self.grid_room = room;
        let french = self.state.language == crate::i18n::Language::French;
        let theme = self.state.theme.unwrap_or(ThemeId::Classic);
        let term_palette = theme.spec().term;
        let font_size = self.state.font_size;
        let term_prefs = self.state.terminal_prefs;

        let project_session_ids: Vec<u64> = self
            .sessions
            .iter()
            .filter(|s| s.project == self.state.active_project)
            .map(|s| s.id)
            .collect();
        if !self.empty_browser_grid()
            && (self.active_session.is_none()
                || !project_session_ids
                    .iter()
                    .any(|id| Some(*id) == self.active_session))
        {
            self.active_session = project_session_ids.first().copied();
        }
        let layout_key = crate::terminal_layout::project_key(self.state.active_project);
        let visible = self.grid_worktree_ids();
        let maximized_id = if project_session_ids.is_empty() {
            None
        } else {
            let layout = self
                .state
                .terminal_layouts
                .entry(layout_key.clone())
                .or_default();
            layout.sync_with_tiling(
                &project_session_ids,
                room,
                self.state.terminal_prefs.tile_new_panes,
            );
            layout
                .maximized
                .filter(|id| visible.as_ref().is_none_or(|ids| ids.contains(id)))
        };

        if self.right_tool_visible(SideTab::Changes)
            && self.scm_commits_project != self.state.active_project
        {
            self.refresh_scm_commits();
        } else if self.right_tool_visible(SideTab::History)
            && self.past_sessions_cache.is_empty()
            && !self.gpui_extra.session_discovery.requested()
            && self.store.is_some()
        {
            self.refresh_past_sessions();
        }
        if self.overlay == Some(Overlay::Settings) {
            match self.current_settings_page_id() {
                "skills" if self.scanned_skills.is_empty() => {
                    self.refresh_scanned_skills();
                }
                "mcp" if self.installed_mcp_servers.is_empty() => {
                    self.refresh_installed_mcp_servers();
                }
                "context"
                    if self.context_scan.total_tokens == 0
                        && self.context_scan.rules_files.is_empty() =>
                {
                    self.refresh_context_scan();
                }
                _ => {}
            }
        }

        let mut any_agent_working = false;
        let waiting = self.sync_waiting_notifications();
        let total_waiting_agents = waiting.len();
        let first_waiting_pane = waiting.first().map(|(id, _)| *id);
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
            if is_agent && is_working {
                any_agent_working = true;
            }
            if s.project == self.state.active_project && is_working {
                active_working_agents += 1;
            }
        }

        let keep_awake_active = self.keep_awake.sync(should_keep_awake(
            self.state.status_bar.keep_awake.into_mode(),
            any_agent_working,
        ));

        let mut tile_targets: Vec<(u64, bool, PxRect)> = Vec::new();
        let mut dividers = Vec::new();
        if let Some(max_sid) = maximized_id.filter(|id| project_session_ids.contains(id)) {
            tile_targets.push((
                max_sid,
                true,
                PxRect {
                    left: 0.0,
                    top: 0.0,
                    width: room[0],
                    height: room[1],
                },
            ));
        } else if let Some(layout) = self.state.terminal_layouts.get(&layout_key) {
            let geom = layout.view_geometry(visible.as_ref());
            let div_px = f64::from(divider_px);
            for tile in &geom.tiles {
                if let Ok(sid) = tile.leaf.id.parse::<u64>() {
                    let rect_px = tile.rect.to_px(room[0], room[1], div_px);
                    tile_targets.push((sid, false, rect_px));
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

        let mut tiles = Vec::with_capacity(tile_targets.len());
        for (sid, maximized, rect_px) in tile_targets {
            if let Some(idx) = self.sessions.iter().position(|s| s.id == sid) {
                // Resize & snapshot terminal if present:
                let term_render = if let Some(term) = &mut self.sessions[idx].terminal {
                    let inner_w = (rect_px.width as f32 - 16.0).max(80.0);
                    let inner_h = (rect_px.height as f32 - 42.0).max(40.0);
                    term.gpui_resize_to_pixels(inner_w, inner_h, font_size, term_prefs.line_height);
                    Some(term.gpui_snapshot(&term_palette))
                } else {
                    None
                };
                let sess = &self.sessions[idx];
                tiles.push(self.build_pane_tile_snapshot(
                    sess,
                    maximized,
                    rect_px,
                    term_render,
                    term_prefs.cursor_style,
                ));
            }
        }

        let floating = {
            let open = self.floating.ui.is_open();
            let height_ratio = (self.floating.ui.height as f32).clamp(0.20, 0.85);
            let phase = self.floating.phase();
            let starting = matches!(
                phase,
                crate::floating_terminal_runtime::Phase::Opening
                    | crate::floating_terminal_runtime::Phase::Restarting
                    | crate::floating_terminal_runtime::Phase::Closing
            );
            let error = self.floating.status_notice(french).map(str::to_owned);
            let title = self.floating.title_str().to_owned();
            let terminal_render = if open {
                let drawer_h = (grid_room_h * height_ratio - 38.0).max(48.0);
                let drawer_w = (grid_room_w - 20.0).max(120.0);
                self.floating.process_mut().map(|term| {
                    term.gpui_resize_to_pixels(
                        drawer_w,
                        drawer_h,
                        font_size,
                        term_prefs.line_height,
                    );
                    term.gpui_snapshot(&term_palette)
                })
            } else {
                None
            };
            GpuiFloatingTerminalSnapshot {
                open,
                height_ratio,
                title,
                starting,
                error,
                terminal_render,
                cursor_style: term_prefs.cursor_style,
            }
        };

        let team_badges = self.teams_hub.session_badges(&self.state);
        let share_snap = self
            .share
            .gpui_snapshot(french, self.overlay == Some(Overlay::Share));
        let joined_guest_initials: Vec<String> = share_snap
            .guests
            .iter()
            .filter(|g| g.joined)
            .map(|g| g.initials.clone())
            .collect();

        let orphan_sessions: Vec<GpuiSessionSidebarRow> = self
            .sessions
            .iter()
            .filter(|s| s.project.is_none())
            .filter(|s| !self.state.sidebar.hide_sleeping || !self.session_is_sleeping(s))
            .map(|s| self.build_sidebar_session_row(s, &team_badges, &joined_guest_initials))
            .collect();

        let projects: Vec<GpuiProjectItem> = self
            .state
            .projects
            .iter()
            .map(|p| {
                let summary = self.worktree_state.summary(p.id);
                let git_branch = summary.map(|s| s.branch.clone()).filter(|b| !b.is_empty());
                let wt_prefs = self.state.project_worktrees.get(&p.id);
                let shown_set: std::collections::HashSet<&str> = wt_prefs
                    .map(|wp| wp.shown_worktrees.iter().map(String::as_str).collect())
                    .unwrap_or_default();
                let inbox_dismissed = wt_prefs.is_some_and(|wp| wp.inbox_dismissed);
                let mut secondary_worktrees = Vec::new();
                let mut visible_worktrees = Vec::new();
                let mut hidden_worktrees = Vec::new();
                if let Some(s) = summary {
                    for wt in s.worktrees.iter().filter(|wt| !wt.is_primary) {
                        secondary_worktrees.push((wt.branch.clone(), wt.path.clone()));
                        let has_open_pane = self.sessions.iter().any(|sess| {
                            sess.project == Some(p.id)
                                && sess
                                    .worktree
                                    .as_ref()
                                    .is_some_and(|sw| sw.path.to_string_lossy() == wt.path)
                        });
                        let is_shown = has_open_pane || shown_set.contains(wt.path.as_str());
                        let row = GpuiWorktreeSidebarRow {
                            branch: wt.branch.clone(),
                            path: wt.path.clone(),
                            dirty: wt.locked,
                            hidden: !is_shown,
                        };
                        if is_shown {
                            visible_worktrees.push(row);
                        } else if !wt.detached {
                            hidden_worktrees.push(row);
                        }
                    }
                }
                let mut sess_count = 0usize;
                let mut running_agents = 0usize;
                let mut waiting_agents = 0usize;
                let mut sessions = Vec::new();
                for s in self.sessions.iter().filter(|s| s.project == Some(p.id)) {
                    sess_count += 1;
                    if self.session_is_running_agent(s.id) {
                        running_agents += 1;
                    }
                    let row =
                        self.build_sidebar_session_row(s, &team_badges, &joined_guest_initials);
                    if row.waiting {
                        waiting_agents += 1;
                    }
                    if !self.state.sidebar.hide_sleeping || !row.sleeping {
                        sessions.push(row);
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
                    sessions,
                    visible_worktrees,
                    hidden_worktrees,
                    inbox_dismissed,
                }
            })
            .collect();

        let paths: Vec<String> = self
            .documents
            .iter()
            .map(|d| d.path.to_string_lossy().into_owned())
            .collect();
        let (labels, guest_labels) = self.document_tab_labels(&paths);
        let mut view_tabs = Vec::with_capacity(self.documents.len() + 2);
        view_tabs.push(GpuiViewTabItem {
            #[cfg(windows)]
            browser_id: None,
            sound: None,
            id: "work".into(),
            title: tr("Terminal").into(),
            hint: self.tab_language().terminal_hint().into(),
            kind: crate::view_tabs::Kind::Terminal,
            doc_index: None,
            guest_key: None,
            selected: self.view == View::Work || self.empty_browser_grid(),
            dirty: false,
            closable: false,
            preview: false,
            deleted: false,
        });
        #[cfg(windows)]
        for (id, title, hint) in self.browser_tab_labels(true) {
            view_tabs.push(GpuiViewTabItem {
                #[cfg(windows)]
                browser_id: Some(id),
                sound: self.browser_speaker(id),
                id: format!("browser:{id}"),
                title,
                hint,
                kind: crate::view_tabs::Kind::Browser,
                doc_index: None,
                guest_key: None,
                selected: self.view == View::Browser && self.browser.tab_id == id,
                dirty: false,
                closable: true,
                preview: false,
                deleted: false,
            });
        }
        for (i, (doc, title)) in self
            .documents
            .iter()
            .zip(labels)
            .enumerate()
            .filter(|(_, (doc, _))| self.visible_document_tab(doc))
        {
            let kind = if doc.diff_title.is_some() {
                crate::view_tabs::Kind::Diff
            } else {
                crate::view_tabs::Kind::Editor
            };
            view_tabs.push(GpuiViewTabItem {
                #[cfg(windows)]
                browser_id: None,
                sound: None,
                id: format!("doc:{i}"),
                title,
                hint: paths[i].clone(),
                kind,
                doc_index: Some(i),
                guest_key: None,
                selected: self.view == View::Files && self.active_document == i,
                dirty: doc.dirty(),
                closable: true,
                preview: doc.unpinned_preview,
                deleted: doc.deleted_on_disk,
            });
        }

        for (key, (title, hint)) in self.share.workspace_tabs().iter().zip(guest_labels) {
            view_tabs.push(GpuiViewTabItem {
                #[cfg(windows)]
                browser_id: None,
                sound: None,
                id: format!(
                    "guest:{}:{}:{}",
                    key.scope.session, key.scope.project, key.path
                ),
                title,
                hint,
                kind: crate::view_tabs::Kind::Editor,
                doc_index: None,
                guest_key: Some(key.clone()),
                selected: self.view == View::GuestFiles && self.share.guest_tab_active(key),
                dirty: self.share.guest_tab_dirty(key),
                closable: true,
                preview: false,
                deleted: false,
            });
        }
        let center_mode = match self.view {
            View::GuestFiles => GpuiCenterMode::GuestFiles,
            View::Files if self.project_document(self.active_document) => {
                GpuiCenterMode::DocumentEditor
            }
            View::Remote => GpuiCenterMode::RemoteSftp,
            #[cfg(windows)]
            View::Browser => GpuiCenterMode::BrowserTab,
            #[cfg(windows)]
            View::EmptyBrowserGrid => GpuiCenterMode::EmptyProjectLauncher,
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

        let active_doc_idx = self.active_document;
        let active_document = if self.gpui_is_document_view() {
            self.documents.get_mut(active_doc_idx).map(|doc| {
                doc.tools.match_case = self.editor_match_case;
                let mut summary =
                    Self::build_document_summary(active_doc_idx, doc, self.state.editor_prefs);
                if !doc.preview_mode && crate::json_outline_panel::supported(doc) {
                    let mut cache = crate::json_outline_panel::Cache::current(
                        self.gpui_extra.outline_cache.take(),
                        doc,
                        self.state.editor_prefs.outline_sort,
                    );
                    summary.breadcrumb_symbols = cache.breadcrumbs(doc);
                    self.gpui_extra.outline_cache = Some(cache);
                }
                summary
            })
        } else {
            None
        };

        self.gpui_extra.disk_comparison_pending = active_document
            .as_ref()
            .and_then(|doc| doc.comparison.as_ref())
            .is_some_and(|comparison| comparison.patch.get().is_none());

        #[cfg(windows)]
        let browser = GpuiBrowserSnapshot::from(&self.browser);
        #[cfg(not(windows))]
        let browser = GpuiBrowserSnapshot::default();

        let preferred_shell_label = match self.state.launcher_prefs.preferred_shell.as_deref() {
            Some("cmd") if cfg!(windows) => "CMD".into(),
            Some("wsl") if cfg!(windows) => "WSL".into(),
            _ if cfg!(windows) => "PowerShell".into(),
            _ => "Shell".into(),
        };
        let installed_agents: Vec<(String, String)> = self
            .agents
            .entries()
            .iter()
            .filter(|(id, installed)| {
                *installed && crate::agent_preferences::enabled(&self.state, id)
            })
            .map(|(id, _)| {
                let label = crate::agent_preferences::display_name(&self.state, id).to_owned();
                (id.clone(), label)
            })
            .collect();
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

        let active_project_name = self.state.active().map(|p| p.name.clone());
        let active_project_path = self
            .state
            .active()
            .map(|p| p.path.to_string_lossy().into_owned());
        let has_active_project = active_project_name.is_some();
        let active_branch = active_pid
            .and_then(|pid| self.worktree_state.summary(pid))
            .map(|s| s.branch.clone())
            .filter(|b| !b.is_empty());

        let telemetry = self.status_telemetry.latest();
        let app_rss_bytes: u64 = telemetry
            .resources
            .as_ref()
            .map(|r| r.app.iter().map(|e| e.memory_bytes as u64).sum())
            .unwrap_or(0);
        let app_rss_mb = app_rss_bytes / (1024 * 1024);
        let listening_ports_count = telemetry.ports.len();

        let sidebar = GpuiSidebarSnapshot {
            prefs: self.state.sidebar,
            collapsed_groups: self.state.collapsed_project_groups.clone(),
            orphan_sessions,
        };
        let side_panels = self.build_side_panels_snapshot(french);
        let settings = (self.overlay == Some(Overlay::Settings))
            .then(|| self.build_settings_snapshot(french, &telemetry));
        let modals = self.build_modals_snapshot(french, &share_snap, &telemetry);

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
            #[cfg(windows)]
            html_toolbar: self.active_html_folder().and_then(|(id, folder)| {
                let (page_path, reload, open) = match &folder {
                    crate::browser::html_trust::Folder::Local(root) => {
                        let doc = self.documents.get(self.active_document)?;
                        let path = doc.path.strip_prefix(root).ok()?;
                        (
                            format!("/{}", path.to_string_lossy().replace('\\', "/")),
                            GpuiAction::ReloadHtmlPreview(id),
                            GpuiAction::OpenHtmlInBrowser(id),
                        )
                    }
                    crate::browser::html_trust::Folder::Ssh { host, root } => {
                        let (_, _, path, _) = self.remote.active_html_page(id)?;
                        let relative = crate::remote::preview_relative(root, path)?;
                        (
                            format!("/{relative}"),
                            GpuiAction::Remote(crate::remote_panel::control::Command::HtmlPage {
                                host: host.clone(),
                                path: path.into(),
                            }),
                            GpuiAction::OpenRemoteHtmlInBrowser {
                                host: host.clone(),
                                path: path.into(),
                            },
                        )
                    }
                };
                Some(HtmlPreviewToolbar {
                    id,
                    page_path,
                    reload,
                    open,
                    decision: if self.editor_page.reconsider == Some(id) {
                        None
                    } else {
                        self.html_trust.decision(&folder)
                    },
                })
            }),
            #[cfg(windows)]
            held_html_link: self.editor_page.external_prompt(),
            theme,
            file_icons: self.state.file_icons,
            dark: self.state.dark,
            french,
            editor_font_size: self.state.editor_prefs.font_size,
            font_size,
            renderer: self.state.renderer,
            #[cfg(windows)]
            side_browser: self
                .selected_side_browser
                .and_then(|id| self.side_browsers.get(&id))
                .map(GpuiBrowserSnapshot::from),
            #[cfg(not(windows))]
            side_browser: None,
            #[cfg(windows)]
            side_browser_tabs: self
                .side_browsers
                .iter()
                .map(|(id, page)| (*id, page.side_tab_title()))
                .collect(),
            #[cfg(not(windows))]
            side_browser_tabs: Vec::new(),
            #[cfg(windows)]
            selected_side_browser: self.selected_side_browser,
            #[cfg(not(windows))]
            selected_side_browser: None,
            right_fullscreen: self.right_fullscreen,
            right_tab: self.state.panels.right_tab,
            center_mode,
            overlay_title,
            toolbar: GpuiToolbarSnapshot {
                left_open: self.state.panels.left_open,
                right_open: self.state.panels.right_open,
                broadcast_enabled: self.state.broadcast,
                floating_open: floating.open,
                share_active: self.overlay == Some(Overlay::Share)
                    || self.share.has_pending_ask()
                    || self.share.is_pill_popover_open()
                    || share_snap.hosting
                    || share_snap.guest_active,
                share_pill_label: share_snap.pill_label,
                share_pill_tooltip: share_snap.pill_tooltip,
                share_pill_badge: share_snap.pill_badge,
                share_pill_dot_live: share_snap.pill_dot_live,
                layout_menu_right: self.gpui_extra.layout_menu_right,
                has_layout_pane: !project_session_ids.is_empty(),
                can_split_shell: self.can_split_shell(!project_session_ids.is_empty()),
                can_apply_grid: !project_session_ids.is_empty() || self.sessions.len() < 100,
                more_menu_open: self.gpui_extra.toolbar_more_open,
                issues_menu_open: self.gpui_extra.issues_menu_open,
                can_arrange_panes: visible
                    .as_ref()
                    .map_or(project_session_ids.len(), |ids| ids.len())
                    >= 2,
                waiting_agents_count: total_waiting_agents,
                first_waiting_pane,
                unread_notifications: self.notifications.unread_count(),
                has_active_project,
                active_project_name: active_project_name.clone(),
                preferred_shell_label,
                installed_agents,
                command_palette_open: self.command_palette.open,
                error_banner: (!self.error.is_empty()).then(|| self.error.clone()),
                build_label: self.dev_update.live_build_label(),
                dev_update_pill: self
                    .dev_update
                    .state()
                    .and_then(crate::dev_update::DevUpdateState::pill_label),
            },
            sidebar,
            projects,
            view_tabs,
            tiles,
            dividers,
            floating,
            active_document,
            browser,
            remote: self.remote.snapshot(&self.state.hosts),
            side_panel: GpuiSidePanelSummary {
                tab: self.state.panels.right_tab,
                changed_files_count,
                staged_count,
                tasks_todo,
                tasks_doing,
                tasks_done,
                history_count: self.state.task_history.records.len(),
                conversations_count: self.activity_store.messages.len(),
                explorer_root_name: active_project_name,
            },
            side_panels,
            settings,
            modals,
            status_bar: GpuiStatusBarSnapshot {
                visible: self.state.status_bar.visible,
                keep_awake: self.state.status_bar.keep_awake,
                keep_awake_active,
                workspace_path: active_project_path,
                git_branch: active_branch,
                session_count: project_session_ids.len(),
                working_agents: active_working_agents,
                ssh_host_count: self.state.hosts.len(),
                listening_ports_count,
                app_rss_mb,
                version_build: format!(
                    "v{} {}",
                    env!("CARGO_PKG_VERSION"),
                    self.dev_update.live_build_label()
                ),
            },
        }
    }

    fn build_document_summary(
        doc_index: usize,
        doc: &mut crate::files::Document,
        preferences: crate::workspace::EditorPreferences,
    ) -> GpuiActiveDocumentSummary {
        let title = doc
            .diff_title
            .clone()
            .or_else(|| {
                doc.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| tr("Editor").into());
        let extension = doc
            .path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let is_diff = doc.diff_title.is_some() && !doc.preview_mode;
        let lang = crate::syntax::SyntaxLang::from_path(&doc.path);
        let can_toggle_preview = doc.diff_title.is_none()
            && doc.pdf.is_none()
            && doc.office.is_none()
            && doc.image.is_none()
            && matches!(
                extension.as_str(),
                "md" | "markdown"
                    | "html"
                    | "htm"
                    | "csv"
                    | "tsv"
                    | "json"
                    | "jsonc"
                    | "jsonl"
                    | "ndjson"
            );
        let display = doc.display_text();
        let line_count = if doc.pdf.is_some() || doc.office.is_some() || doc.image.is_some() {
            0
        } else {
            display.line_count
        };

        let matches = if doc.tools.find_open && !doc.tools.find_query.is_empty() {
            crate::files::find_char_matches(&doc.text, &doc.tools.find_query, doc.tools.match_case)
        } else {
            Vec::new()
        };
        let match_count = matches.len();
        let match_index = if match_count == 0 {
            0
        } else {
            doc.tools.match_index.min(match_count - 1)
        };
        let match_lines: Vec<usize> = matches.iter().map(|m| m.2).collect();
        let scroll_to_line = doc.tools.pending_scroll_line;

        let sheet = if doc.preview_mode && matches!(extension.as_str(), "csv" | "tsv") {
            if doc.sheet_cache.is_none() {
                let explicit = (extension == "tsv").then_some(b'\t');
                doc.sheet_cache = Some(std::sync::Arc::new(
                    crate::rich_previews::parse_delimited_sheet(&doc.text, explicit),
                ));
            }
            doc.sheet_cache.clone()
        } else {
            None
        };

        let json_preview = if doc.preview_mode
            && matches!(extension.as_str(), "json" | "jsonc" | "jsonl" | "ndjson")
        {
            crate::rich_previews::parse_json_or_jsonl(&doc.text, &extension).ok()
        } else {
            None
        };

        let readable_blocks = if doc.tools.html_plain_text.is_none()
            && doc.preview_mode
            && matches!(extension.as_str(), "html" | "htm")
        {
            if doc.preview.is_none() {
                doc.preview = Some(crate::preview::Preview::parse(&doc.text, false));
            }
            doc.preview
                .as_ref()
                .map(crate::preview::Preview::blocks)
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let pdf_summary = doc.pdf.as_mut().map(crate::pdf::PdfView::gpui_poll_summary);
        let office_summary = doc
            .office
            .as_mut()
            .map(crate::office::OfficeView::gpui_poll_summary);
        let image_dims = doc.image.as_ref().map(|img| img.size);
        if doc.gpui_image.is_none() {
            doc.gpui_image = doc
                .image
                .as_ref()
                .and_then(crate::gpui_pixels::from_color_image);
        }
        let media_pixels = doc.gpui_image.clone();

        GpuiActiveDocumentSummary {
            instance_id: doc.instance_id(),
            line_numbers: preferences.line_numbers,
            syntax_highlighting: preferences.syntax_highlighting,
            doc_index,
            title,
            conflict_disk: doc.tools.conflict_disk.clone(),
            comparison: doc.comparison().cloned(),
            deleted_on_disk: doc.deleted_on_disk,
            disk_reload: doc.tools.disk_reload,
            breadcrumb_path: {
                let path = dunce::simplified(&doc.path);
                path.strip_prefix(dunce::simplified(&doc.root))
                    .unwrap_or(path)
                    .components()
                    .filter(|part| {
                        !matches!(
                            part,
                            std::path::Component::RootDir | std::path::Component::CurDir
                        )
                    })
                    .map(|part| part.as_os_str().to_string_lossy().into_owned())
                    .collect()
            },
            breadcrumb_symbols: Default::default(),
            path_display: dunce::simplified(&doc.path).to_string_lossy().into_owned(),
            external_available: doc.external_available(),
            extension,
            lang,
            is_diff,
            dirty: display.dirty,
            preview_mode: doc.preview_mode,
            can_toggle_preview,
            media: doc.media,
            word_wrap: if is_diff {
                preferences.diff_word_wrap
            } else {
                preferences.word_wrap
            },
            find_open: doc.tools.find_open,
            replace_open: doc.tools.replace_open,
            goto_open: doc.tools.goto_open,
            find_query: doc.tools.find_query.clone(),
            match_case: doc.tools.match_case,
            replace_text: doc.tools.replace_text.clone(),
            goto_line: doc.tools.goto_line.clone(),
            match_count,
            match_index,
            match_lines,
            hunk_lines: display.hunk_lines,
            scroll_to_line,
            line_count,
            text: display.text,
            force_large_markdown_render: doc.force_large_markdown_render,
            sheet,
            json_preview,
            html_plain_text: doc.tools.html_plain_text.clone(),
            readable_blocks,
            pdf_summary,
            office_summary,
            html_link_menu: doc
                .tools
                .html_link_menu
                .as_ref()
                .map(|(pos, _, web)| (*pos, *web)),
            image_menu: doc.tools.image_menu,
            image_download_notice: doc.tools.image_download_notice.clone(),
            image_dims,
            playback: doc.playback.clone(),
            media_pixels,
            error: doc.error.clone(),
        }
    }

    fn build_pane_tile_snapshot(
        &self,
        sess: &Session,
        maximized: bool,
        rect_px: PxRect,
        terminal_render: Option<crate::terminal::TerminalRenderSnapshot>,
        cursor_style: crate::workspace::TerminalCursorStyle,
    ) -> GpuiPaneTileSnapshot {
        let is_agent = matches!(
            sess.launch,
            Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
        );
        let is_native_chat = sess.launch.native_chat().is_some();
        let can_switch_chat_mode = is_native_chat
            || matches!(&sess.launch, Launch::Agent(a) if matches!(a.as_str(), "codex" | "claude"));
        let dormant = !is_native_chat
            && sess.terminal.is_none()
            && sess.loading.is_none()
            && sess.profile_migration.is_none()
            && sess.restarting.is_none();
        let kind_label = match &sess.launch {
            Launch::Shell => "Shell".into(),
            Launch::CodexChat => "Codex Chat".into(),
            Launch::ClaudeChat => "Claude Chat".into(),
            Launch::Agent(agent) => agent.clone(),
            Launch::Ssh(host) => format!("SSH · {}", host.target.alias),
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
        } else if sess.loading.is_some() || sess.profile_migration.is_some() {
            vec![tr("Starting session…").into()]
        } else {
            Vec::new()
        };
        let native_chat = match &sess.launch {
            Launch::CodexChat => self
                .chats
                .iter()
                .find(|c| c.pane == sess.id)
                .map(crate::chat::Chat::gpui_snapshot),
            Launch::ClaudeChat => self
                .claude_chats
                .iter()
                .find(|c| c.pane == sess.id)
                .map(crate::claude::Chat::gpui_snapshot),
            _ => None,
        };

        let team_badge = self.teams_hub.session_badges(&self.state).remove(&sess.id);
        let share_guests = if self.active_session == Some(sess.id) {
            self.share
                .gpui_snapshot(false, false)
                .guests
                .into_iter()
                .filter(|g| g.joined)
                .map(|g| g.initials)
                .collect()
        } else {
            Vec::new()
        };
        let wake_banner = self.teams_hub.wake_banners.get(&sess.id).cloned();

        GpuiPaneTileSnapshot {
            exit_banner: sess.terminal.as_ref().filter(|t| t.exited()).map(|t| {
                sess.exit_banner(
                    t.exit_code(),
                    self.state.language == crate::i18n::Language::French,
                )
            }),
            session_id: sess.id,
            title: sess.label().to_owned(),
            kind_label,
            is_agent,
            is_native_chat,
            can_switch_chat_mode,
            dormant,
            active: self.active_session == Some(sess.id),
            maximized,
            broadcast: sess.broadcast,
            loading: (sess.loading.is_some() || sess.profile_migration.is_some())
                || sess.restarting.is_some(),
            exited: sess.terminal.as_ref().is_some_and(|t| t.exited()),
            model_chip,
            account_chip,
            worktree_branch: sess.worktree.as_ref().map(|w| w.branch.clone()),
            cwd_display: sess.cwd.to_string_lossy().into_owned(),
            rect_px,
            preview_lines,
            terminal_render,
            cursor_style,
            native_chat,
            team_badge,
            share_guests,
            wake_banner,
            error: sess.error.clone(),
        }
    }

    fn session_is_sleeping(&self, sess: &Session) -> bool {
        let is_native_chat = sess.launch.native_chat().is_some();
        !is_native_chat
            && sess.terminal.is_none()
            && sess.loading.is_none()
            && sess.profile_migration.is_none()
            && sess.restarting.is_none()
    }

    fn build_sidebar_session_row(
        &self,
        sess: &Session,
        team_badges: &std::collections::HashMap<u64, crate::teams_relay::SessionTeamBadge>,
        joined_guest_initials: &[String],
    ) -> GpuiSessionSidebarRow {
        let is_agent = matches!(
            sess.launch,
            Launch::Agent(_) | Launch::CodexChat | Launch::ClaudeChat
        );
        let sleeping = self.session_is_sleeping(sess);
        let working = self.session_is_running_agent(sess.id);
        let waiting = match &sess.launch {
            Launch::CodexChat => self
                .chats
                .iter()
                .any(|c| c.pane == sess.id && c.waiting_for_user()),
            Launch::ClaudeChat => self
                .claude_chats
                .iter()
                .any(|c| c.pane == sess.id && c.waiting_for_user()),
            _ => false,
        };
        let exited = sess.terminal.as_ref().is_some_and(|t| t.exited());
        let kind_label = match &sess.launch {
            Launch::Shell => "Shell".into(),
            Launch::CodexChat => "Codex Chat".into(),
            Launch::ClaudeChat => "Claude Chat".into(),
            Launch::Agent(agent) => agent.clone(),
            Launch::Ssh(host) => format!("SSH · {}", host.target.alias),
        };
        let task_line = if self.state.sidebar.hide_agent_task_line || !is_agent {
            None
        } else {
            sess.terminal
                .as_ref()
                .and_then(|t| {
                    t.output()
                        .lines()
                        .rev()
                        .map(str::trim)
                        .find(|l| !l.is_empty())
                        .map(|l| l.chars().take(64).collect::<String>())
                })
                .or_else(|| sess.worktree.as_ref().map(|w| format!("⎇ {}", w.branch)))
        };
        let share_guests = if self.active_session == Some(sess.id) {
            joined_guest_initials.to_vec()
        } else {
            Vec::new()
        };
        GpuiSessionSidebarRow {
            id: sess.id,
            project_id: sess.project,
            title: sess.label().to_owned(),
            kind_label,
            is_agent,
            active: self.active_session == Some(sess.id),
            sleeping,
            waiting,
            working,
            exited,
            worktree_branch: sess.worktree.as_ref().map(|w| w.branch.clone()),
            task_line,
            team_badge: team_badges.get(&sess.id).cloned(),
            share_guests,
        }
    }

    fn build_side_panels_snapshot(&mut self, french: bool) -> GpuiSidePanelsSnapshot {
        let visible = self
            .state
            .panels
            .right_open
            .then_some(self.state.panels.right_tab);
        if visible == Some(SideTab::Changes) {
            self.sync_git_project();
        }
        use crate::side_panels_model::{
            build_dashboard, filter_and_sort_task_history, now_unix_ms,
        };

        let dashboard_vm = if visible == Some(SideTab::Dashboard) {
            let dash_items = self.build_dashboard_items();
            build_dashboard(
                &dash_items,
                self.side_panels_ui.dashboard_group,
                self.side_panels_ui.dashboard_filter,
                &self.side_panels_ui.dashboard_query,
                french,
            )
        } else {
            Default::default()
        };

        let active_root = self.state.active().map(|p| p.path.clone());
        let active_doc_path = (self.view == View::Files)
            .then(|| {
                self.documents
                    .get(self.active_document)
                    .map(|d| d.path.clone())
            })
            .flatten();
        let explorer = if visible == Some(SideTab::Files) {
            self.explorer_tree
                .gpui_rows_snapshot(active_root.as_deref(), active_doc_path.as_deref())
        } else {
            Default::default()
        };

        let (scm_branch, scm_ahead, scm_behind, scm_entries, scm_error) =
            match &self.scm_state.result {
                Some(Ok(parsed)) => (
                    parsed.branch.clone(),
                    parsed.ahead,
                    parsed.behind,
                    if visible == Some(SideTab::Changes) {
                        parsed.entries.clone()
                    } else {
                        Vec::new()
                    },
                    None,
                ),
                Some(Err(err)) => (None, 0, 0, Vec::new(), Some(err.clone())),
                None => (None, 0, 0, Vec::new(), None),
            };

        let active_pid = self.state.active_project;
        let tasks: Vec<crate::workspace::Task> = active_pid
            .filter(|_| visible == Some(SideTab::Tasks))
            .map(|pid| {
                self.state
                    .tasks
                    .iter()
                    .filter(|t| t.project == pid)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let proj_filter = if self.side_panels_ui.history_current_project_only {
            active_pid
        } else {
            None
        };
        let task_history_rows = if visible == Some(SideTab::TaskHistory) {
            filter_and_sort_task_history(
                &self.state.task_history.records,
                proj_filter,
                self.side_panels_ui.history_window,
                self.side_panels_ui.history_sort,
                &self.side_panels_ui.history_query,
                now_unix_ms(),
            )
            .0
        } else {
            Vec::new()
        };

        let workspace_roots: Vec<PathBuf> =
            self.state.projects.iter().map(|p| p.path.clone()).collect();
        let agent_filter = (!self.side_panels_ui.sessions_agent_filter.is_empty())
            .then_some(self.side_panels_ui.sessions_agent_filter.as_str());
        let sessions_visible =
            visible == Some(SideTab::History) || self.side_panels_ui.sessions_modal_open;
        let past_sessions = if sessions_visible {
            self.gpui_extra
                .session_query
                .update(
                    crate::session_query::Key {
                        entries: self.past_sessions_cache.clone(),
                        scope: self.side_panels_ui.sessions_scope,
                        active: active_root,
                        roots: workspace_roots,
                        agent: agent_filter.map(str::to_owned),
                        query: self.side_panels_ui.sessions_query.clone(),
                        transcripts: self.side_panels_ui.sessions_include_transcripts,
                    },
                    std::time::Instant::now(),
                )
                .entries
                .clone()
        } else {
            self.gpui_extra.session_query.pause();
            Vec::new()
        };
        let sessions_loading =
            self.gpui_extra.session_query.pending() || self.side_panels_ui.sessions_loading;
        let sessions_limited = sessions_visible
            && (self.gpui_extra.session_query.limited() || self.side_panels_ui.sessions_limited);

        let threads = if visible == Some(SideTab::Conversations) {
            let preview = self.conversation_preview();
            self.gpui_extra.conversation_model.update(
                &self.activity_store,
                &preview,
                self.gpui_extra.conversation_loader.revision,
                self.side_panels_ui.conv_window,
                self.side_panels_ui.conv_media,
                &self.side_panels_ui.conv_query,
                now_unix_ms(),
            )
        } else {
            std::sync::Arc::default()
        };

        let outline_index = self.local_explorer_document_index();
        if self
            .gpui_extra
            .outline_query
            .as_ref()
            .is_some_and(|(id, _)| {
                outline_index
                    .and_then(|i| self.documents.get(i))
                    .is_none_or(|d| d.instance_id() != *id)
                    || visible != Some(SideTab::Files)
                    || !self.gpui_extra.outline_open
            })
        {
            self.gpui_extra.outline_query = None;
        }
        let query = self
            .gpui_extra
            .outline_query
            .as_ref()
            .map(|(_, q)| q.clone());
        let outline = if visible == Some(SideTab::Files)
            && self.gpui_extra.outline_open
            && let Some(index) = outline_index
        {
            self.documents
                .get_mut(index)
                .filter(|doc| crate::json_outline_panel::supported(doc))
                .map(|doc| {
                    let mut cache = crate::json_outline_panel::Cache::current(
                        self.gpui_extra.outline_cache.take(),
                        doc,
                        self.state.editor_prefs.outline_sort,
                    );
                    if query.as_deref().is_none_or(|q| q.trim().is_empty())
                        && cache.follow(doc, self.state.editor_prefs.outline_follow_cursor)
                    {
                        doc.tools.outline_scroll_generation =
                            doc.tools.outline_scroll_generation.wrapping_add(1);
                    }
                    let symbols = cache.symbols.clone();
                    self.gpui_extra.outline_cache = Some(cache);
                    (doc.instance_id(), symbols)
                })
        } else {
            None
        };
        let editor_uses_symbols = self.gpui_is_document_view()
            && self
                .documents
                .get(self.active_document)
                .is_some_and(|doc| !doc.preview_mode && crate::json_outline_panel::supported(doc));
        if outline.is_none() && !editor_uses_symbols {
            self.gpui_extra.outline_cache = None;
        }
        let mut open_editors = Vec::new();
        if visible == Some(SideTab::Files) && self.state.active_project.is_some() {
            for (i, doc) in self
                .documents
                .iter()
                .enumerate()
                .filter(|(_, d)| d.project == self.state.active_project)
            {
                let id = doc.instance_id();
                let parent = doc
                    .path
                    .parent()
                    .map(dunce::simplified)
                    .unwrap_or(std::path::Path::new(""));
                let folder = self
                    .state
                    .active()
                    .and_then(|project| parent.strip_prefix(dunce::simplified(&project.path)).ok())
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .unwrap_or(parent);
                open_editors.push(ExplorerOpenEditor {
                    folder: folder.display().to_string(),
                    title: doc.diff_title.clone().unwrap_or_else(|| {
                        doc.path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    }),
                    path: doc.path.to_string_lossy().into_owned(),
                    selected: self.view == View::Files && self.active_document == i,
                    dirty: doc.diff_title.is_none() && doc.dirty(),
                    preview: doc.unpinned_preview,
                    select: GpuiAction::ExplorerActivateDocument(id),
                    close: GpuiAction::ExplorerCloseDocument(id),
                });
            }
            #[cfg(windows)]
            for (id, title, path) in self.browser_labels() {
                open_editors.push(ExplorerOpenEditor {
                    folder: path.clone(),
                    title,
                    path,
                    selected: self.view == View::Browser && self.browser.tab_id == id,
                    dirty: false,
                    preview: false,
                    select: GpuiAction::SelectBrowserPage(id),
                    close: GpuiAction::CloseBrowserPage(id),
                });
            }
        }
        GpuiSidePanelsSnapshot {
            open_editors,
            editor_project: self.state.active_project,
            open_editors_shown: self.state.editor_prefs.open_editors_shown,
            folder_shown: self.state.editor_prefs.explorer_folder_shown,
            outline_height: self
                .state
                .editor_prefs
                .explorer_outline_height
                .clamp(44, 2000),
            timeline_height: self
                .state
                .editor_prefs
                .explorer_timeline_height
                .clamp(44, 2000),
            explorer_editable: self.file_actions_ready(),
            explorer_can_insert: self.explorer_terminal_id().is_some(),
            timeline: self.timeline_snapshot(visible == Some(SideTab::Files)),
            outline_placeholder: self
                .gpui_extra
                .outline_cache
                .as_ref()
                .map_or("", |c| c.placeholder(french)),
            outline_folds: outline
                .as_ref()
                .and_then(|_| outline_index.and_then(|i| self.documents.get(i)))
                .map(|d| d.tools.outline_folds.clone())
                .unwrap_or_default(),
            outline_open: self.gpui_extra.outline_open,
            outline_options_open: self.gpui_extra.outline_options_open,
            outline_query: query,
            outline_filter_on_type: self.state.editor_prefs.outline_filter_on_type,
            outline_sort: self.state.editor_prefs.outline_sort,
            outline_follow_cursor: self.state.editor_prefs.outline_follow_cursor,
            outline_scroll_generation: outline_index
                .and_then(|i| self.documents.get(i))
                .map_or(0, |d| d.tools.outline_scroll_generation),
            outline_selected: outline_index
                .and_then(|i| self.documents.get(i))
                .and_then(|doc| doc.tools.outline_selected.clone()),
            outline,
            dashboard_vm,
            dashboard_group: self.side_panels_ui.dashboard_group,
            dashboard_filter: self.side_panels_ui.dashboard_filter,
            dashboard_query: self.side_panels_ui.dashboard_query.clone(),
            explorer,
            scm_branch,
            scm_ahead: scm_ahead as usize,
            scm_behind: scm_behind as usize,
            scm_busy: self.commit_in_flight || self.git_result.busy(),
            scm_commit_busy: self.gpui_scm_busy(),
            scm_commit_msg: self
                .gpui_scm_draft()
                .map(|v| v.2)
                .unwrap_or_default()
                .to_owned(),
            scm_entries,
            scm_error,
            commits_open: self.side_panels_ui.commits_open,
            commits: if visible == Some(SideTab::Changes) {
                self.scm_commits_cache.clone()
            } else {
                Vec::new()
            },
            expanded_commits: self.side_panels_ui.expanded_commits.clone(),
            commit_files: if visible == Some(SideTab::Changes) {
                self.side_panels_ui.commit_files_cache.clone()
            } else {
                BTreeMap::new()
            },
            tasks,
            new_task_title: self.gpui_extra.new_task_title.clone(),
            task_agents: active_pid
                .map(|p| self.task_agent_choices(p))
                .unwrap_or_default(),
            task_board: self.gpui_extra.task_board.clone(),
            history_window: self.side_panels_ui.history_window,
            history_sort: self.side_panels_ui.history_sort,
            history_current_project_only: self.side_panels_ui.history_current_project_only,
            history_query: self.side_panels_ui.history_query.clone(),
            task_history_rows,
            sessions_scope: self.side_panels_ui.sessions_scope,
            sessions_group: self.side_panels_ui.sessions_group,
            sessions_query: self.side_panels_ui.sessions_query.clone(),
            past_sessions,
            sessions_loading,
            sessions_include_transcripts: self.side_panels_ui.sessions_include_transcripts,
            sessions_limited,
            conv_window: self.side_panels_ui.conv_window,
            conv_query: self.side_panels_ui.conv_query.clone(),
            selected_thread_key: self.side_panels_ui.selected_thread_key.clone(),
            threads,
            conversations_loading: self.gpui_extra.conversation_loader.loading(),
            conversations_limited: self.gpui_extra.conversation_loader.limited,
        }
    }

    fn build_settings_snapshot(
        &mut self,
        french: bool,
        telemetry: &crate::status_metrics::TelemetryReport,
    ) -> GpuiSettingsSnapshot {
        use crate::settings_model::{ALL_SETTINGS_GROUPS, SettingsLang};

        let lang = if french {
            SettingsLang::Fr
        } else {
            SettingsLang::En
        };
        let mut groups = Vec::new();
        for group in ALL_SETTINGS_GROUPS {
            let mut pages = Vec::new();
            for page in group.pages() {
                if !cfg!(windows)
                    && matches!(page, crate::settings_model::SettingsPageId::TesselAccount)
                {
                    continue;
                }
                let title = page.title(lang);
                pages.push((page.as_str().to_owned(), title.to_owned()));
            }
            if !pages.is_empty() {
                groups.push((group.title(lang).to_owned(), pages));
            }
        }

        let mut running_agents = 0usize;
        let mut sleeping_sessions = 0usize;
        for s in &self.sessions {
            if self.session_is_running_agent(s.id) {
                running_agents += 1;
            }
            if self.session_is_sleeping(s) {
                sleeping_sessions += 1;
            }
        }
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
        let app_rss_bytes: u64 = telemetry
            .resources
            .as_ref()
            .map(|r| r.app.iter().map(|e| e.memory_bytes as u64).sum())
            .unwrap_or(0);
        let panes_rss_bytes: u64 = telemetry
            .resources
            .as_ref()
            .map(|r| r.terminals.values().map(|e| e.memory_bytes as u64).sum())
            .unwrap_or(0);
        let panes_cpu_pct: f32 = telemetry
            .resources
            .as_ref()
            .map(|r| r.terminals.values().map(|e| e.cpu_percent as f32).sum())
            .unwrap_or(0.0);

        let stats_snapshot = crate::settings_pages_panel::StatsSnapshot {
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
            listening_ports: telemetry.ports.len(),
        };

        let usage_summary = crate::dialogs_toolbar_model::compute_usage_summary(
            &self.state.task_history.records,
            10.0,
            crate::dialogs_toolbar_model::now_ms(),
        );

        let mut builtin_agents: Vec<BuiltinAgentSettingRow> = Vec::new();
        let mut builtin_agent_details: Vec<GpuiBuiltinAgentDetail> = Vec::new();
        for entry in crate::agent_catalog::ALL {
            let installed = self
                .agents
                .entries()
                .iter()
                .any(|(id, inst)| id == entry.id && *inst);
            let enabled = crate::agent_preferences::enabled(&self.state, entry.id);
            let prefs = crate::agent_preferences::get(&self.state, entry.id)
                .cloned()
                .unwrap_or_default();
            let env_error = crate::agent_env::parse(&prefs.env).err();
            let listed_models_caption = self.model_discovery.as_ref().and_then(|d| {
                let models = d.models(entry.id)?;
                let meta = d.lists().get(entry.id);
                let date = meta
                    .and_then(|e| crate::regional_date::local(e.fetched_at, french))
                    .unwrap_or_else(|| "—".into());
                Some(crate::agent_model_defaults_ui::format_models_listed(
                    models.len(),
                    entry.name,
                    &date,
                    french,
                ))
            });
            let can_refresh_models =
                crate::agent_model_catalog::find(entry.id).is_some_and(|c| c.probed);
            builtin_agents.push((
                entry.id.to_owned(),
                entry.name.to_owned(),
                installed,
                enabled,
            ));
            builtin_agent_details.push(GpuiBuiltinAgentDetail {
                id: entry.id.to_owned(),
                name: entry.name.to_owned(),
                installed,
                enabled,
                command: prefs.command,
                args: prefs.args,
                env: prefs.env,
                env_error,
                listed_models_caption,
                can_refresh_models,
            });
        }

        let provider_secrets: Vec<ProviderSecretRow> =
            crate::settings_extra_model::PROVIDER_SECRET_KEYS
                .iter()
                .map(|&k| {
                    let stored = self.provider_secret_flags.get(k).copied().unwrap_or(false);
                    (k.to_owned(), k.to_owned(), stored)
                })
                .collect();

        #[cfg(windows)]
        let vault = (self.settings_section == SettingsSection::Passwords)
            .then(|| self.vault.gpui_snapshot(&self.browser));
        #[cfg(not(windows))]
        let vault = None;

        #[cfg(windows)]
        let account = (self.settings_section == SettingsSection::Account)
            .then(|| self.account.gpui_snapshot());
        #[cfg(not(windows))]
        let account = None;

        let active_page_id = match self.settings_section {
            #[cfg(windows)]
            SettingsSection::Passwords => "passwords".into(),
            _ => self.current_settings_page_id().to_owned(),
        };

        GpuiSettingsSnapshot {
            active_page_id,
            search_query: self.gpui_extra.settings_search.clone(),
            groups,
            general_prefs: self.state.general_prefs.clone(),
            preferred_shell: self.state.launcher_prefs.preferred_shell.clone(),
            wsl_available: self.wsl_available,
            terminal_prefs: self.state.terminal_prefs,
            editor_prefs: self.state.editor_prefs,
            browser_prefs: self.state.browser_prefs,
            git_prefs: self.state.git_prefs.clone(),
            git_username: self.git_username.clone(),
            stats_prefs: self.state.stats_prefs,
            stats_snapshot,
            usage_summary,
            quick_commands: self.state.quick_commands.clone(),
            new_quick_cmd_name: self.gpui_extra.new_quick_cmd_name.clone(),
            new_quick_cmd_text: self.gpui_extra.new_quick_cmd_text.clone(),
            new_quick_cmd_enter: self.gpui_extra.new_quick_cmd_enter,
            can_import_ssh: self.store.is_some(),
            importing_ssh: self.host_import.is_some(),
            ssh_tests: self.host_tests.clone(),
            ssh_notice: self.host_notice.clone(),
            ssh_hosts: self.state.hosts.clone(),
            new_ssh_alias: self.gpui_extra.new_ssh_alias.clone(),
            new_ssh_folder: self.gpui_extra.new_ssh_folder.clone(),
            editing_ssh: self.gpui_extra.editing_ssh.clone(),
            builtin_agents,
            builtin_agent_details,
            agent_folder_trust: self.state.agent_folder_trust,
            expanded_agent_id: self.gpui_extra.expanded_agent_id.clone(),
            custom_agent_error: self.settings_extra.custom_error.clone(),
            custom_agents: self.state.custom_agents.clone(),
            new_custom_agent_name: self.gpui_extra.new_custom_agent_name.clone(),
            new_custom_agent_cmd: self.gpui_extra.new_custom_agent_cmd.clone(),
            new_custom_agent_accent: self.gpui_extra.new_custom_agent_accent,
            claude_accounts: self.claude_managed_accounts.clone(),
            codex_accounts: self.codex_managed_accounts.clone(),
            claude_system_email: self.claude_system_email.clone(),
            codex_system_email: self.codex_system_email.clone(),
            new_account_label: self.gpui_extra.new_account_label.clone(),
            provider_secrets,
            secret_key_idx: self.gpui_extra.secret_key_idx,
            secret_value_draft: self.gpui_extra.secret_value_draft.clone(),
            cli_sign_ins: self.cli_sign_ins.clone(),
            skills: self.scanned_skills.clone(),
            skills_git_url: self.gpui_extra.skills_git_url.clone(),
            installed_mcp: self.installed_mcp_servers.clone(),
            mcp_probe_outcomes: self.settings_extra.mcp_probe_outcomes.clone(),
            mcp_card_messages: self.settings_extra.mcp_card_messages.clone(),
            mcp_test_status: self.settings_extra.mcp_test_status.clone(),
            context_prefs: self.state.context_prefs,
            context_scan: self.context_scan.clone(),
            orchestration_prefs: self.state.orchestration_prefs.clone(),
            notification_prefs: self.state.notification_prefs.clone(),
            voice_prefs: self.state.voice_prefs.clone(),
            devtools: self.devtools_statuses.clone(),
            automations: self.automations_store.clone(),
            new_auto_name: self.gpui_extra.new_auto_name.clone(),
            new_auto_prompt: self.gpui_extra.new_auto_prompt.clone(),
            new_auto_every_mins: self.gpui_extra.new_auto_every_mins,
            vault,
            account,
            backup_status: self.backup_status.clone(),
        }
    }

    fn build_modals_snapshot(
        &self,
        french: bool,
        share_snap: &crate::share_panel::ShareGpuiSnapshot,
        telemetry: &crate::status_metrics::TelemetryReport,
    ) -> GpuiModalsSnapshot {
        let palette_rows = if self.command_palette.open {
            let current_theme = self.state.theme.unwrap_or(ThemeId::Classic);
            let cmds = self.build_palette_commands(current_theme);
            crate::palette::filter_palette_commands(&cmds, &self.command_palette.query)
        } else {
            Vec::new()
        };

        let has_panes = self
            .sessions
            .iter()
            .any(|s| s.project == self.state.active_project);
        let auto_tile = self.state.terminal_prefs.tile_new_panes;
        let launch_placements = crate::finder_model::available_placements(has_panes, auto_tile);
        let target_pane_name = self
            .active_session
            .and_then(|id| self.sessions.iter().find(|s| s.id == id))
            .map(Session::label);
        let launch_lang = if french {
            crate::finder_model::LauncherLang::Fr
        } else {
            crate::finder_model::LauncherLang::En
        };
        let launch_where_hint = crate::finder_model::where_text(
            self.gpui_extra.launch_placement,
            has_panes,
            auto_tile,
            target_pane_name,
            launch_lang,
        );

        let help_open = self.overlay == Some(Overlay::Help);
        let help_groups = if help_open {
            crate::help_panel::filter_supported_shortcuts(
                french,
                super::SUPPORTED_HELP_SHORTCUT_KEYS,
                &self.gpui_extra.help_query,
            )
        } else {
            Vec::new()
        };
        let help_footer = crate::help_panel::help_footer_text(french).to_owned();

        let add_nested_repos = self
            .dialogs_toolbar_ui
            .nested_repos
            .iter()
            .map(|p| {
                (
                    p.clone(),
                    self.dialogs_toolbar_ui.selected_nested_repos.contains(p),
                )
            })
            .collect();
        let add_ssh_aliases = self.state.hosts.iter().map(|h| h.alias.clone()).collect();
        let add_folder_name = match self.dialogs_toolbar_ui.add_tab {
            crate::dialogs_toolbar_ui::AddProjectTab::CloneRepo => {
                self.dialogs_toolbar_ui.clone_folder_name.clone()
            }
            _ => self.dialogs_toolbar_ui.init_folder_name.clone(),
        };

        let review_candidates = if self.side_panels_ui.review_modal_open {
            self.review_candidate_branches()
        } else {
            Vec::new()
        };

        let teams_project = self.teams_ui.manager_project;
        let teams_list = teams_project
            .map(|pid| {
                self.state
                    .teams
                    .iter()
                    .filter(|t| t.project == pid)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let teams_project_panes = teams_project
            .map(|pid| {
                self.sessions
                    .iter()
                    .filter(|s| s.project == Some(pid))
                    .map(|s| (s.id, s.label().to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let teams_pending_workers = self
            .teams_hub
            .workers
            .iter()
            .filter(|w| w.state == crate::teams_relay::WorkerState::Confirming)
            .cloned()
            .collect();

        let usage_summary = crate::dialogs_toolbar_model::compute_usage_summary(
            &self.state.task_history.records,
            10.0,
            crate::dialogs_toolbar_model::now_ms(),
        );

        let telemetry_app_rss_bytes: u64 = telemetry
            .resources
            .as_ref()
            .map(|r| r.app.iter().map(|e| e.memory_bytes as u64).sum())
            .unwrap_or(0);
        let telemetry_panes_rss_bytes: u64 = telemetry
            .resources
            .as_ref()
            .map(|r| r.terminals.values().map(|e| e.memory_bytes as u64).sum())
            .unwrap_or(0);
        let telemetry_panes_cpu_pct: f32 = telemetry
            .resources
            .as_ref()
            .map(|r| r.terminals.values().map(|e| e.cpu_percent as f32).sum())
            .unwrap_or(0.0);
        let telemetry_app_sparkline = telemetry
            .sparklines
            .samples(crate::sidebar_model::APP_HISTORY_KEY)
            .to_vec();

        let prompt_modal = if let Some((_, draft, _)) = &self.rename_project_modal {
            Some(GpuiPromptModalSnapshot {
                title: tr("Rename project").into(),
                subtitle: tr("Enter a new display name for this workspace:").into(),
                draft: draft.clone(),
                confirm_label: tr("Rename").into(),
            })
        } else if let Some(ra) = &self.rename_agent {
            Some(GpuiPromptModalSnapshot {
                title: tr("Rename pane").into(),
                subtitle: tr("Enter a custom label for this pane:").into(),
                draft: ra.draft.clone(),
                confirm_label: tr("Save").into(),
            })
        } else if self.dialogs_toolbar_ui.group_modal_project.is_some() {
            Some(GpuiPromptModalSnapshot {
                title: if french {
                    "Groupe de projets".into()
                } else {
                    "Project folder group".into()
                },
                subtitle: if french {
                    "Nom du dossier (laisser vide pour retirer du groupe) :".into()
                } else {
                    "Folder group name (leave empty to ungroup):".into()
                },
                draft: self.dialogs_toolbar_ui.group_modal_draft.clone(),
                confirm_label: tr("Save").into(),
            })
        } else if let Some(fa) = &self.file_action {
            match fa.kind {
                crate::file_actions::Kind::File
                | crate::file_actions::Kind::Folder
                | crate::file_actions::Kind::Rename => Some(GpuiPromptModalSnapshot {
                    title: match fa.kind {
                        crate::file_actions::Kind::File => tr("New file").into(),
                        crate::file_actions::Kind::Folder => tr("New folder").into(),
                        _ => tr("Rename").into(),
                    },
                    subtitle: fa.path.to_string_lossy().into_owned(),
                    draft: fa.name.clone(),
                    confirm_label: tr("Confirm").into(),
                }),
                _ => None,
            }
        } else {
            None
        };

        let confirm_modal = if let Some(doc_idx) = self.pending_close {
            let doc_name = self
                .documents
                .get(doc_idx)
                .and_then(|d| d.path.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "document".into());
            Some(GpuiConfirmModalSnapshot {
                title: tr("Unsaved changes").into(),
                body: format!("{doc_name} — {}", tr("Save changes before closing?")),
                confirm_label: tr("Save & close").into(),
                secondary_label: Some(tr("Discard").into()),
                danger: false,
            })
        } else if let Some(sid) = self.pending_close_session {
            let pane_lbl = self
                .sessions
                .iter()
                .find(|s| s.id == sid)
                .map_or("pane", Session::label);
            Some(GpuiConfirmModalSnapshot {
                title: tr("Close running agent?").into(),
                body: format!("{pane_lbl} — {}", tr("Terminate this active session?")),
                confirm_label: tr("Close pane").into(),
                secondary_label: None,
                danger: true,
            })
        } else if let Some(pid) = self.pending_remove_project {
            let p_name = self
                .state
                .projects
                .iter()
                .find(|p| p.id == pid)
                .map_or("project", |p| p.name.as_str());
            Some(GpuiConfirmModalSnapshot {
                title: tr("Remove project from Tessel?").into(),
                body: format!("{p_name} — {}", tr("Files on disk will not be deleted.")),
                confirm_label: tr("Remove").into(),
                secondary_label: None,
                danger: true,
            })
        } else if let Some((_, _, copy)) = &self.pending_scm_discard {
            Some(GpuiConfirmModalSnapshot {
                title: copy.title.clone(),
                body: copy.description.clone(),
                confirm_label: copy.confirm_label.clone(),
                secondary_label: None,
                danger: true,
            })
        } else if let Some((_, wt_path, branch)) = &self.pending_delete_worktree {
            Some(GpuiConfirmModalSnapshot {
                title: tr("Delete Git worktree?").into(),
                body: format!("{branch} ({})", wt_path.display()),
                confirm_label: tr("Delete worktree").into(),
                secondary_label: None,
                danger: true,
            })
        } else if let Some((ids, group)) = &self.gpui_extra.task_board.pending_delete {
            let title = if *group {
                match (french, ids.len()) {
                    (true, 1) => "Supprimer 1 tâche ?".into(),
                    (true, n) => format!("Supprimer {n} tâches ?"),
                    (false, 1) => "Delete 1 task?".into(),
                    (false, n) => format!("Delete {n} tasks?"),
                }
            } else {
                let name = self
                    .state
                    .tasks
                    .iter()
                    .find(|t| ids.contains(&t.id))
                    .map(|t| t.title.as_str())
                    .unwrap_or_default();
                if french {
                    format!("Supprimer « {name} » ?")
                } else {
                    format!("Delete \"{name}\"?")
                }
            };
            Some(GpuiConfirmModalSnapshot {
                title,
                body: match (french, group) {
                    (true, true) => "Les cartes sont retirées du tableau.",
                    (true, false) => "La carte est retirée du tableau.",
                    (false, true) => "The cards are removed from the board.",
                    (false, false) => "The card is removed from the board.",
                }
                .into(),
                confirm_label: if french { "Supprimer" } else { "Delete" }.into(),
                secondary_label: None,
                danger: false,
            })
        } else if let Some(entry) = &self.pending_transcript_trash {
            Some(GpuiConfirmModalSnapshot {
                title: tr("Move conversation to Recycle Bin?").into(),
                body: format!(
                    "{}\n{}",
                    entry.title,
                    tr(
                        "The local transcript will be removed from history. Close its pane before continuing."
                    )
                ),
                confirm_label: tr("Move to Recycle Bin").into(),
                secondary_label: None,
                danger: true,
            })
        } else if let Some(spec) = &self.pending_agent_install {
            Some(GpuiConfirmModalSnapshot {
                title: spec.title.clone(),
                body: format!("{}\n{}", spec.description, spec.code),
                confirm_label: spec.confirm_label.clone(),
                secondary_label: None,
                danger: false,
            })
        } else if let Some(fa) = &self.file_action {
            match fa.kind {
                crate::file_actions::Kind::Delete | crate::file_actions::Kind::Trash => {
                    Some(GpuiConfirmModalSnapshot {
                        title: tr("Delete path?").into(),
                        body: fa.path.to_string_lossy().into_owned(),
                        confirm_label: tr("Delete").into(),
                        secondary_label: None,
                        danger: true,
                    })
                }
                _ => None,
            }
        } else {
            None
        };

        GpuiModalsSnapshot {
            palette_open: self.command_palette.open,
            palette_query: self.command_palette.query.clone(),
            palette_selected: self.command_palette.index,
            palette_rows,
            finder: self.finder.gpui_snapshot(),
            finder_can_insert: self.finder_target().is_some(),
            launch_menu_open: self.gpui_extra.launch_menu_open
                || self.overlay == Some(Overlay::Launcher),
            launch_placement: self.gpui_extra.launch_placement,
            launch_placements,
            launch_where_hint,
            launch_in_worktree: self.gpui_extra.launch_in_worktree,
            help_open,
            help_query: self.gpui_extra.help_query.clone(),
            help_groups,
            help_footer,
            share_modal_open: self.overlay == Some(Overlay::Share),
            share: share_snap.clone(),
            add_project_open: self.dialogs_toolbar_ui.add_project_open,
            add_project_tab: self.dialogs_toolbar_ui.add_tab,
            add_picked_folder: self.dialogs_toolbar_ui.picked_folder.clone(),
            add_nested_repos,
            add_clone_url: self.dialogs_toolbar_ui.clone_url.clone(),
            add_clone_parent: self.dialogs_toolbar_ui.clone_parent.clone(),
            add_folder_name,
            add_ssh_aliases,
            add_ssh_selected: self.dialogs_toolbar_ui.ssh_selected_alias.clone(),
            add_ssh_remote_dir: self.dialogs_toolbar_ui.ssh_remote_dir.clone(),
            add_error: self.dialogs_toolbar_ui.add_error.clone(),
            github_open: self.dialogs_toolbar_ui.github_open,
            github_tab: self.dialogs_toolbar_ui.github_tab,
            github_query: self.dialogs_toolbar_ui.github_query.clone(),
            github_prs: self.dialogs_toolbar_ui.github_prs.clone(),
            github_issues: self.dialogs_toolbar_ui.github_issues.clone(),
            github_error: self.dialogs_toolbar_ui.github_error.clone(),
            notes_open: self.side_panels_ui.notes_modal_open,
            sessions_open: self.side_panels_ui.sessions_modal_open,
            notes_text: self.side_panels_ui.notes.text.clone(),
            notes_dirty: self.side_panels_ui.notes.is_dirty(),
            review_open: self.side_panels_ui.review_modal_open,
            review_candidates,
            review_head_branch: self.side_panels_ui.review_head_branch.clone(),
            review_snapshot: self.side_panels_ui.review_snapshot.clone(),
            review_selected_file: self.side_panels_ui.review_selected_file.clone(),
            review_selected_diff: self.side_panels_ui.review_selected_diff.clone(),
            review_error: self.side_panels_ui.review_error.clone(),
            worktree_cleanup: self.worktree_cleanup.clone(),
            teams_manager_project: teams_project,
            teams_new_name: self.teams_ui.new_team_name.clone(),
            teams_list,
            teams_project_panes,
            teams_workers: self.teams_hub.workers.clone(),
            teams_pending_workers,
            teams_relay_prompts: self.teams_hub.approval_prompts.iter().cloned().collect(),
            usage_popover_open: self.gpui_extra.usage_popover_open,
            usage_summary,
            notifications_popover_open: self.gpui_extra.notifications_popover_open,
            notifications_items: self.notifications.items.clone(),
            status_popover: self.gpui_extra.status_popover,
            telemetry_app_rss_bytes,
            telemetry_panes_rss_bytes,
            telemetry_panes_cpu_pct,
            telemetry_app_sparkline,
            telemetry_ports: telemetry.ports.clone(),
            prompt_modal,
            confirm_modal,
            dev_update_open: self.dev_update_dialog_open,
            dev_update_state: self.dev_update.state().cloned(),
            dev_update_busy: self.dev_update.is_busy(),
            dev_update_notice: self.dev_update_notice.clone(),
        }
    }

    fn with_target_terminal(
        &mut self,
        session_id: Option<u64>,
        f: impl FnOnce(&mut crate::terminal::Terminal),
    ) {
        match session_id {
            Some(sid) if self.view == View::Work => {
                if let Some(term) = self
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == sid && s.project == self.state.active_project)
                    .and_then(|s| s.terminal.as_mut())
                {
                    f(term);
                }
            }
            Some(_) => {}
            None => {
                if let Some(term) = self.floating.process_mut() {
                    f(term);
                }
            }
        }
    }

    fn reconcile_native_task_board(&mut self) {
        let project = self
            .right_tool_visible(SideTab::Tasks)
            .then_some(self.state.active_project)
            .flatten()
            .filter(|id| self.state.projects.iter().any(|p| p.id == *id));
        self.gpui_extra
            .task_board
            .reconcile(project, &self.state.tasks);
    }
    pub(crate) fn gpui_cancel_task_selection(&mut self) -> bool {
        self.reconcile_native_task_board();
        if self.gpui_has_open_modal() || !self.gpui_extra.task_board.selecting {
            return false;
        }
        if let Some(scope) = self.gpui_extra.task_board.scope {
            self.apply_gpui_action(GpuiAction::NativeTaskBoard {
                scope,
                command: crate::native_task_board::Command::ToggleSelection,
            });
            return true;
        }
        false
    }
    pub(crate) fn gpui_task_rename_input(
        &self,
    ) -> Option<(crate::native_task_board::Scope, u64, &str)> {
        if self.gpui_has_open_modal() {
            return None;
        }
        let board = &self.gpui_extra.task_board;
        Some((
            board.scope?,
            board.edit.as_ref()?.serial,
            &board.edit.as_ref()?.draft,
        ))
    }
    pub(crate) fn apply_gpui_action(&mut self, action: GpuiAction) {
        self.reconcile_native_task_board();
        let overlay = self.overlay;
        let explorer = self.gpui_explorer_owner();
        let panel_query = self.gpui_panel_query_context();
        if !matches!(action, GpuiAction::OpenHtmlLinkMenuTarget(_))
            && let Some(doc) = self.documents.get_mut(self.active_document)
        {
            doc.tools.html_link_menu = None;
        }
        self.apply_gpui_action_inner(action);
        self.reconcile_native_task_board();
        if panel_query != self.gpui_panel_query_context() {
            self.gpui_extra.panel_query_generation = self
                .gpui_extra
                .panel_query_generation
                .checked_add(1)
                .expect("panel query identity exhausted");
        }
        if explorer != self.gpui_explorer_owner() {
            self.gpui_extra.explorer_input_generation = self
                .gpui_extra
                .explorer_input_generation
                .checked_add(1)
                .expect("explorer input identity exhausted");
        }
        if overlay != self.overlay {
            if overlay == Some(Overlay::Settings) {
                self.gpui_extra.settings_search.clear();
            }
            self.gpui_extra.modal_input_generation = self
                .gpui_extra
                .modal_input_generation
                .checked_add(1)
                .expect("modal input identity exhausted");
        }
    }
    fn apply_gpui_action_inner(&mut self, action: GpuiAction) {
        self.ensure_native_chat_instances();
        match action {
            GpuiAction::Remote(command) => {
                if self.view == View::Remote {
                    self.remote.command(command, &self.state.hosts);
                    self.open_requested_remote_terminal();
                }
            }
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
            GpuiAction::NewSideBrowser => {
                #[cfg(windows)]
                self.new_side_browser();
            }
            GpuiAction::DockBrowser(id) => {
                #[cfg(windows)]
                self.dock_browser(id);
                #[cfg(not(windows))]
                let _ = id;
            }
            GpuiAction::UndockBrowserToProject { page, project } => {
                #[cfg(windows)]
                if !self.gpui_has_open_modal()
                    && self.side_browsers.contains_key(&page)
                    && self.state.projects.iter().any(|item| item.id == project)
                {
                    self.select_workspace_project(project);
                    self.undock_browser(page);
                }
                #[cfg(not(windows))]
                let _ = (page, project);
            }
            GpuiAction::UndockBrowserAt {
                page,
                anchor,
                before,
            } => {
                #[cfg(windows)]
                self.undock_browser_at(page, anchor, before);
                #[cfg(not(windows))]
                let _ = (page, anchor, before);
            }
            GpuiAction::UndockBrowserOnPane { page, pane } => {
                #[cfg(windows)]
                self.undock_browser_on_pane(page, pane);
                #[cfg(not(windows))]
                let _ = (page, pane);
            }
            GpuiAction::UndockBrowser(id) => {
                #[cfg(windows)]
                self.undock_browser(id);
                #[cfg(not(windows))]
                let _ = id;
            }
            GpuiAction::SelectSideBrowser(id) => {
                #[cfg(windows)]
                self.select_side_browser(id);
                #[cfg(not(windows))]
                let _ = id;
            }
            GpuiAction::CloseSideBrowser(id) => {
                #[cfg(windows)]
                self.close_side_browser(id);
                #[cfg(not(windows))]
                let _ = id;
            }
            GpuiAction::SelectSideTab(tab) => {
                self.toggle_right_tool(tab);
                self.save();
            }
            GpuiAction::ToggleRightFullscreen => {
                if !self.state.panels.right_open {
                    self.state.panels.right_open = true;
                }
                self.right_fullscreen = !self.right_fullscreen;
            }
            GpuiAction::SelectProject(id) => self.select_workspace_project(id),
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
                self.split_shell_beside(target_id, dir);
            }
            GpuiAction::SplitActiveShell(dir) => self.split_active_shell(dir),
            GpuiAction::LaunchDefaultOrShell => {
                let launch = self.default_new_launch();
                self.new_session_placed(launch, None);
            }
            GpuiAction::LaunchAgent(agent_id) => {
                self.new_session_placed(Launch::Agent(agent_id), None);
            }
            GpuiAction::LaunchCodexChat => {
                if self.state.active().is_some() {
                    self.new_session_placed(Launch::CodexChat, None);
                    self.ensure_native_chat_instances();
                } else {
                    self.error = tr("Select a project for native chat.").into();
                }
            }
            GpuiAction::LaunchClaudeChat => {
                if self.state.active().is_some() {
                    self.new_session_placed(Launch::ClaudeChat, None);
                    self.ensure_native_chat_instances();
                } else {
                    self.error = tr("Select a project for native chat.").into();
                }
            }
            GpuiAction::FocusSession(id) => {
                if self.sessions.iter().any(|s| s.id == id) {
                    if self.active_session != Some(id) {
                        self.active_session = Some(id);
                        self.save();
                    }
                    self.view = View::Work;
                }
            }
            GpuiAction::CloseSession(id) => {
                self.close_session(id);
            }
            GpuiAction::WakeSession(id) => {
                let preferred_shell = self.state.launcher_prefs.preferred_shell.clone();
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
                if let Some(sess) = self.sessions.iter_mut().find(|s| s.id == id) {
                    sess.start_with_shell(prefs, preferred_shell);
                }
                self.active_session = Some(id);
            }
            GpuiAction::RestartSession(id) => {
                let preferred_shell = self.state.launcher_prefs.preferred_shell.clone();
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
                if let Some(sess) = self.sessions.iter_mut().find(|s| s.id == id) {
                    sess.retry_terminal(prefs, preferred_shell);
                }
            }
            GpuiAction::SwitchSessionChatMode(pane_id) => {
                self.switch_session_chat_mode(pane_id);
                self.ensure_native_chat_instances();
            }
            GpuiAction::ToggleMaximizeSession(id) => {
                if self.view != View::Work
                    || !self
                        .sessions
                        .iter()
                        .any(|s| s.id == id && s.project == self.state.active_project)
                {
                    return;
                }
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
                if self.state.broadcast
                    && let Some(sess) = self
                        .sessions
                        .iter_mut()
                        .find(|s| s.id == id && s.project == self.state.active_project)
                {
                    sess.broadcast = !sess.broadcast;
                    self.save();
                }
            }
            GpuiAction::ToggleBroadcastMode => {
                self.state.broadcast = !self.state.broadcast;
                self.save();
            }
            GpuiAction::LayoutMenu(right) => {
                self.gpui_extra.layout_menu_right = right;
            }
            GpuiAction::ChooseLayout(action) => {
                if self.gpui_extra.layout_menu_right.take().is_some() {
                    self.apply_layout_action(action);
                    self.save();
                }
            }
            GpuiAction::MovePane { from, to, zone } => {
                let visible = self.grid_worktree_ids();
                let key = crate::terminal_layout::project_key(self.state.active_project);
                if self.view == View::Work
                    && !self.gpui_has_open_modal()
                    && let Some(layout) = self.state.terminal_layouts.get_mut(&key)
                    && visible.as_ref().is_none_or(|ids| {
                        ids.contains(&from) && to.is_none_or(|id| ids.contains(&id))
                    })
                    && layout.maximized.is_none()
                    && layout.move_pane(from, to, zone)
                {
                    if let Some(members) = &visible {
                        layout.mark_view_manual(members);
                    }
                    self.active_session = Some(from);
                    self.save();
                }
            }
            GpuiAction::ResizeSplitDivider {
                split_id,
                dir: _,
                index,
                split_side_px,
                delta_px,
            } => {
                let key = crate::terminal_layout::project_key(self.state.active_project);
                let visible = self.grid_worktree_ids();
                if let Some(layout) = self.state.terminal_layouts.get_mut(&key)
                    && layout.resize_view(
                        visible.as_ref(),
                        &split_id,
                        index,
                        split_side_px,
                        delta_px,
                        None,
                    )
                {
                    self.save();
                }
            }
            GpuiAction::SelectWorkTab => {
                self.select_work_tab();
            }
            GpuiAction::SelectDocumentTab(idx) => {
                if self.project_document(idx) {
                    self.active_document = idx;
                    self.view = View::Files;
                }
            }
            GpuiAction::KeepDocumentTab(idx) => {
                if self
                    .documents
                    .get(idx)
                    .is_some_and(|doc| self.visible_document_tab(doc))
                    && let Some(doc) = self.documents.get_mut(idx)
                {
                    doc.unpinned_preview = false;
                }
            }
            GpuiAction::CloseDocumentTab(idx) => {
                if self
                    .documents
                    .get(idx)
                    .is_some_and(|doc| self.visible_document_tab(doc))
                {
                    self.request_document_close(idx);
                }
            }
            GpuiAction::SelectBrowserTab => {
                #[cfg(windows)]
                {
                    self.browser_tab_open = true;
                    self.view = View::Browser;
                }
            }
            #[cfg(windows)]
            GpuiAction::SelectBrowserPage(id) => self.select_browser_page(id),
            #[cfg(windows)]
            GpuiAction::CloseBrowserPage(id) => self.close_browser_page(id),
            GpuiAction::ToggleBrowserSound(id) => {
                #[cfg(windows)]
                self.toggle_browser_sound(id);
                #[cfg(not(windows))]
                let _ = id;
            }
            GpuiAction::CloseBrowserTab => {
                #[cfg(windows)]
                self.close_browser_page(self.browser.tab_id);
            }
            GpuiAction::ToggleFloatingTerminal => {
                self.toggle_floating_headless();
            }
            GpuiAction::RestartFloatingTerminal => {
                self.restart_floating_headless();
            }
            GpuiAction::CloseFloatingTerminal => {
                self.close_floating_headless();
            }
            GpuiAction::SetFloatingHeightRatio(ratio) => {
                self.set_floating_height_headless(ratio);
            }
            GpuiAction::TerminalScroll {
                session_id,
                delta_lines,
            } => {
                self.with_target_terminal(session_id, |t| t.gpui_scroll_lines(delta_lines));
            }
            GpuiAction::TerminalBackToLive(session_id) => {
                self.with_target_terminal(session_id, crate::terminal::Terminal::gpui_back_to_live);
            }
            GpuiAction::TerminalPaste { session_id, text } => {
                let confirm = self.state.terminal_prefs.confirm_multiline_paste;
                self.with_target_terminal(session_id, |t| t.gpui_paste_with_prefs(&text, confirm));
                self.flush_gpui_terminal_side_effects();
            }
            GpuiAction::TerminalConfirmMultilinePaste {
                session_id,
                confirm,
            } => {
                self.with_target_terminal(session_id, |t| t.gpui_confirm_multiline_paste(confirm));
                self.flush_gpui_terminal_side_effects();
            }
            GpuiAction::TerminalOpenFind(session_id) => {
                self.with_target_terminal(session_id, crate::terminal::Terminal::open_find);
            }
            GpuiAction::TerminalCloseFind(session_id) => {
                self.with_target_terminal(session_id, crate::terminal::Terminal::gpui_close_find);
            }
            GpuiAction::ApplyNativeTerminalFindEdit {
                session_id,
                instance_id,
                base,
                text,
            } => {
                if text.len() <= 65536
                    && self.gpui_terminal_find_input(session_id)
                        == Some((instance_id, base.as_str()))
                    && text != base
                {
                    self.with_target_terminal(session_id, |t| t.gpui_find_set_query(text));
                }
            }
            GpuiAction::NativeTerminalFindCommand {
                session_id,
                instance_id,
                query,
                step,
            } => {
                if self.gpui_terminal_find_input(session_id) == Some((instance_id, query.as_str()))
                {
                    self.with_target_terminal(session_id, |t| {
                        if let Some(delta) = step {
                            t.gpui_find_step(delta)
                        } else {
                            t.gpui_close_find()
                        }
                    });
                }
            }
            GpuiAction::TerminalDismissDeviceLogin(session_id) => {
                self.with_target_terminal(
                    session_id,
                    crate::terminal::Terminal::gpui_dismiss_device_login,
                );
            }
            GpuiAction::TerminalOpenTesselUrl(url) => {
                self.open_tessel_url_headless(url);
            }
            GpuiAction::ChatConnect(pane_id) => {
                if let Some(c) = self.chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_connect();
                } else if let Some(c) = self.claude_chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_connect();
                }
            }
            GpuiAction::ChatStop(pane_id) => {
                self.cancel_owner_task_pane(pane_id);
                if let Some(c) = self.chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_stop();
                } else if let Some(c) = self.claude_chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_stop();
                }
            }
            GpuiAction::ChatInterrupt(pane_id) => {
                let provider = self
                    .sessions
                    .iter()
                    .find(|s| s.id == pane_id)
                    .map(|s| &s.launch);
                match provider {
                    Some(Launch::CodexChat) => {
                        if let Some(chat) = self.chats.iter_mut().find(|c| c.pane == pane_id) {
                            chat.gpui_interrupt();
                        }
                    }
                    Some(Launch::ClaudeChat) => {
                        if let Some(chat) = self.claude_chats.iter_mut().find(|c| c.pane == pane_id)
                        {
                            chat.gpui_interrupt();
                        }
                    }
                    _ => return,
                }
                self.cancel_owner_task_pane(pane_id);
            }
            GpuiAction::ChatNewConversation(pane_id) => {
                self.cancel_owner_task_pane(pane_id);
                if let Some(c) = self.claude_chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_new_conversation();
                }
            }
            GpuiAction::ChatLoadOlder(pane_id) => {
                if let Some(c) = self.chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_load_older();
                }
            }
            GpuiAction::ApplyNativeChatEdit {
                pane_id,
                instance_id,
                base,
                text,
            } => {
                if self.gpui_chat_input_allowed(pane_id)
                    && crate::chat::message_fits(&text)
                    && self.gpui_chat_composer(pane_id) == Some((instance_id, base.as_str()))
                {
                    if let Some(c) = self
                        .chats
                        .iter_mut()
                        .find(|c| c.pane == pane_id && c.composer_state().0 == instance_id)
                    {
                        c.gpui_set_composer(text);
                    } else if let Some(c) = self
                        .claude_chats
                        .iter_mut()
                        .find(|c| c.pane == pane_id && c.composer_state().0 == instance_id)
                    {
                        c.gpui_set_composer(text);
                    }
                }
            }
            GpuiAction::NativeChatSend {
                pane_id,
                instance_id,
                text,
            } => {
                if self.gpui_chat_input_allowed(pane_id)
                    && self.gpui_chat_composer(pane_id) == Some((instance_id, text.as_str()))
                {
                    if let Some(c) = self
                        .chats
                        .iter_mut()
                        .find(|c| c.pane == pane_id && c.composer_state().0 == instance_id)
                    {
                        c.gpui_send_message();
                    } else if let Some(c) = self
                        .claude_chats
                        .iter_mut()
                        .find(|c| c.pane == pane_id && c.composer_state().0 == instance_id)
                    {
                        c.gpui_send_message();
                    }
                }
            }
            GpuiAction::ChatSetModel { pane_id, model } => {
                if let Some(c) = self.chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_set_model(model);
                } else if let Some(c) = self.claude_chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_set_model(model);
                }
                self.save();
            }
            GpuiAction::ChatSetEffort { pane_id, effort } => {
                if let Some(c) = self.chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_set_effort(effort);
                } else if let Some(c) = self.claude_chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_set_effort(effort);
                }
                self.save();
            }
            GpuiAction::ChatSetResume { pane_id, resume } => {
                if let Some(c) = self.chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.set_resume(resume);
                } else if let Some(c) = self.claude_chats.iter_mut().find(|c| c.pane == pane_id) {
                    c.gpui_set_resume(resume);
                }
            }
            GpuiAction::ChatRespondApproval {
                pane_id,
                request_instance_id,
                allow,
            } => {
                if !self.gpui_chat_input_allowed(pane_id)
                    || self.gpui_chat_request_id(pane_id, true) != Some(request_instance_id)
                {
                    return;
                }
                if let Some(c) = self.chats.iter_mut().find(|c| {
                    c.pane == pane_id && c.approval_instance_id() == Some(request_instance_id)
                }) {
                    c.gpui_respond_approval(allow);
                } else if let Some(c) = self.claude_chats.iter_mut().find(|c| {
                    c.pane == pane_id && c.approval_instance_id() == Some(request_instance_id)
                }) {
                    c.gpui_respond_approval(allow);
                }
            }
            GpuiAction::ChatSelectQuestionOption {
                pane_id,
                request_instance_id,
                q_idx,
                opt_idx,
            } => {
                if !self.gpui_chat_input_allowed(pane_id)
                    || self.gpui_chat_request_id(pane_id, false) != Some(request_instance_id)
                {
                    return;
                }
                if let Some(c) = self.chats.iter_mut().find(|c| {
                    c.pane == pane_id && c.question_instance_id() == Some(request_instance_id)
                }) {
                    c.gpui_select_question_option(q_idx, opt_idx);
                } else if let Some(c) = self.claude_chats.iter_mut().find(|c| {
                    c.pane == pane_id && c.question_instance_id() == Some(request_instance_id)
                }) {
                    c.gpui_select_question_option(q_idx, opt_idx);
                }
            }
            GpuiAction::ApplyNativeQuestionEdit {
                pane_id,
                request_instance_id,
                q_idx,
                base,
                text,
            } => {
                if text == base
                    || !self.gpui_chat_input_allowed(pane_id)
                    || text.chars().take(4097).count() > 4096
                    || self.gpui_chat_question_state(pane_id, q_idx)
                        != Some((request_instance_id, base.as_str()))
                {
                    return;
                }
                if let Some(c) = self.chats.iter_mut().find(|c| {
                    c.pane == pane_id && c.question_instance_id() == Some(request_instance_id)
                }) {
                    c.gpui_set_question_text(q_idx, text);
                } else if let Some(c) = self.claude_chats.iter_mut().find(|c| {
                    c.pane == pane_id && c.question_instance_id() == Some(request_instance_id)
                }) {
                    c.gpui_set_question_text(q_idx, text);
                }
            }
            GpuiAction::ChatSubmitQuestions {
                pane_id,
                request_instance_id,
                skip,
            } => {
                if !self.gpui_chat_input_allowed(pane_id)
                    || self.gpui_chat_request_id(pane_id, false) != Some(request_instance_id)
                {
                    return;
                }
                if let Some(c) = self.chats.iter_mut().find(|c| {
                    c.pane == pane_id && c.question_instance_id() == Some(request_instance_id)
                }) {
                    c.gpui_submit_questions(skip);
                } else if let Some(c) = self.claude_chats.iter_mut().find(|c| {
                    c.pane == pane_id && c.question_instance_id() == Some(request_instance_id)
                }) {
                    c.gpui_submit_questions(skip);
                }
            }
            GpuiAction::OpenChatCitation(pane, path) => self.open_chat_citation(pane, &path),
            GpuiAction::OpenDocumentPath(path) => {
                if let Some(pid) = self.state.active_project
                    && let Some(root) = self
                        .state
                        .projects
                        .iter()
                        .find(|p| p.id == pid)
                        .map(|p| p.path.clone())
                {
                    match crate::files::Document::open(&root, &path) {
                        Ok(mut doc) => {
                            doc.project = Some(pid);
                            self.documents.push(doc);
                            self.active_document = self.documents.len() - 1;
                            self.view = View::Files;
                        }
                        Err(e) => self.error = e,
                    }
                }
            }
            GpuiAction::SelectGuestDocument(key) => {
                if self.share.select_workspace_file(key) {
                    self.overlay = None;
                    self.view = View::GuestFiles;
                }
            }
            GpuiAction::SaveGuestDocument(key) => {
                if self
                    .gpui_guest_editor()
                    .is_some_and(|doc| doc.key == key && doc.dirty())
                {
                    self.share.save_guest_file(&key);
                }
            }
            GpuiAction::RecoverGuestFile(key, action) => {
                if self.overlay == Some(Overlay::Share)
                    && let Some(text) = self.share.recover_file(&key, action)
                {
                    self.gpui_extra.copied_clipboard = Some(text);
                }
            }
            GpuiAction::CopyGuestFileText(key) => {
                if (self.overlay == Some(Overlay::Share)
                    && self.share.tab == crate::share_panel::DialogTab::Join)
                    || (self.overlay.is_none() && self.view == View::GuestFiles)
                {
                    self.gpui_extra.copied_clipboard = self.share.current_guest_file_text(&key);
                }
            }
            GpuiAction::OpenEditorExternally(id) => self.open_active_file_externally(id),
            GpuiAction::ResolveDiskConflict(id, expected, action) => {
                if self.gpui_is_document_view()
                    && !self.gpui_has_open_modal()
                    && let Some(doc) = self.documents.get_mut(self.active_document)
                    && doc.instance_id() == id
                    && let Err(error) = doc.resolve_conflict(&expected, action)
                {
                    doc.error = error;
                }
            }
            GpuiAction::SaveActiveDocument => {
                if let Some(doc) = self.documents.get_mut(self.active_document) {
                    match doc.save(self.store.as_ref().map(|s| s.directory())) {
                        Ok(()) => doc.error.clear(),
                        Err(e) => doc.error = e,
                    }
                }
            }
            #[cfg(windows)]
            GpuiAction::ReloadHtmlPreview(id) => {
                if self.view == View::Files
                    && self.gpui_media_unobscured()
                    && let Some(doc) = self.documents.get(self.active_document)
                    && doc.instance_id() == id
                    && doc.preview_mode
                    && doc.tools.html_plain_text.is_none()
                    && let Err(error) = self.editor_page.reload(doc)
                {
                    self.error = error;
                }
            }
            #[cfg(windows)]
            GpuiAction::OpenRemoteHtmlInBrowser {
                host: expected,
                path,
            } => {
                if self.view == View::Remote
                    && !self.gpui_has_open_modal()
                    && let Some((host, root, text)) = self.remote.html_preview(&path)
                    && host.connection_key() == expected
                {
                    let folder = crate::browser::html_trust::Folder::Ssh {
                        host: host.connection_key(),
                        root: root.into(),
                    };
                    let scripts = self.html_trust.decision(&folder) == Some(true);
                    match self
                        .browser
                        .open_remote_page(host, root, &path, text, scripts)
                    {
                        Ok(()) => {
                            self.show_agent_browser = false;
                            self.browser_tab_open = true;
                            self.view = View::Browser;
                        }
                        Err(error) => self.error = error,
                    }
                }
            }
            #[cfg(windows)]
            GpuiAction::OpenHtmlInBrowser(id) => {
                if self.view == View::Files
                    && !self.gpui_has_open_modal()
                    && let Some(doc) = self.documents.get(self.active_document)
                    && doc.instance_id() == id
                    && doc.diff_title.is_none()
                    && !doc.media
                {
                    let scripts = doc.tools.html_root.as_ref().is_some_and(|root| {
                        self.html_trust
                            .decision(&crate::browser::html_trust::Folder::Local(root.clone()))
                            == Some(true)
                    });
                    match self.browser.open_project_page(doc, scripts) {
                        Ok(()) => {
                            self.show_agent_browser = false;
                            self.browser_tab_open = true;
                            self.view = View::Browser;
                        }
                        Err(err) => self.error = err,
                    }
                }
            }
            GpuiAction::SetHtmlView(id, mode) => {
                if self.view == View::Files
                    && let Some(doc) = self.documents.get_mut(self.active_document)
                    && doc.instance_id() == id
                    && !doc.media
                    && doc.diff_title.is_none()
                    && doc
                        .path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| {
                            ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")
                        })
                {
                    doc.preview_mode = mode != HtmlViewMode::Source;
                    doc.tools.html_plain_text = (mode == HtmlViewMode::Plain)
                        .then(|| crate::preview::html_plain_text(&doc.text).into());
                }
            }
            GpuiAction::ToggleActiveDocumentPreview => {
                if let Some(doc) = self.documents.get_mut(self.active_document) {
                    doc.tools.html_plain_text = None;
                    doc.preview_mode = !doc.preview_mode;
                }
            }
            GpuiAction::ForceRenderLargeMarkdown => {
                if let Some(doc) = self.documents.get_mut(self.active_document) {
                    doc.force_large_markdown_render = true;
                    doc.preview_mode = true;
                }
            }
            GpuiAction::ToggleEditorWordWrap => {
                if self.gpui_editor_document().is_some() {
                    self.state.editor_prefs.word_wrap = !self.state.editor_prefs.word_wrap;
                    self.save();
                }
            }
            GpuiAction::ToggleEditorFind => {
                if let Some(doc) = self.documents.get_mut(self.active_document) {
                    doc.tools.find_open = !doc.tools.find_open;
                    if !doc.tools.find_open {
                        doc.tools.replace_open = false;
                    }
                }
            }
            GpuiAction::ToggleEditorReplace => {
                if let Some(doc) = self.documents.get_mut(self.active_document)
                    && !doc.media
                {
                    doc.tools.replace_open = !doc.tools.replace_open;
                    if doc.tools.replace_open {
                        doc.tools.find_open = true;
                    }
                }
            }
            GpuiAction::ToggleEditorGoto => {
                if let Some(doc) = self.documents.get_mut(self.active_document) {
                    doc.tools.goto_open = !doc.tools.goto_open;
                }
            }
            GpuiAction::ToggleEditorMatchCase => {
                if self.gpui_editor_tool_document_id().is_some()
                    && let Some(doc) = self.documents.get_mut(self.active_document)
                {
                    doc.toggle_find_case();
                    let enabled = doc.tools.match_case;
                    self.remember_editor_match_case(enabled);
                }
            }
            GpuiAction::SetEditorFindQuery(q) => {
                if let Some(doc) = self.documents.get_mut(self.active_document) {
                    doc.tools.find_query = q;
                    doc.tools.match_index = 0;
                    doc.tools.pending_char_range = None;
                    doc.tools.pending_scroll_line = None;
                    let hits = crate::files::find_char_matches(
                        &doc.text,
                        &doc.tools.find_query,
                        doc.tools.match_case,
                    );
                    if let Some(first) = hits.first() {
                        doc.tools.pending_char_range = Some((first.0, first.1));
                        doc.tools.pending_scroll_line = Some(first.2);
                    }
                }
            }
            GpuiAction::SetEditorReplaceText(r) => {
                if let Some(doc) = self.documents.get_mut(self.active_document) {
                    doc.tools.replace_text = r;
                }
            }
            GpuiAction::StepEditorFind(dir) => {
                if let Some(doc) = self.documents.get_mut(self.active_document) {
                    let hits = crate::files::find_char_matches(
                        &doc.text,
                        &doc.tools.find_query,
                        doc.tools.match_case,
                    );
                    if !hits.is_empty() {
                        let len = hits.len();
                        doc.tools.match_index = if dir >= 0 {
                            (doc.tools.match_index + 1) % len
                        } else {
                            (doc.tools.match_index + len - 1) % len
                        };
                        doc.tools.pending_scroll_line = Some(hits[doc.tools.match_index].2);
                        let hit = hits[doc.tools.match_index];
                        doc.tools.pending_char_range = Some((hit.0, hit.1));
                    }
                }
            }
            action @ (GpuiAction::ReplaceEditorCurrent | GpuiAction::ReplaceEditorAll) => {
                if let Some(doc) = self.documents.get_mut(self.active_document)
                    && !doc.media
                {
                    let hits = crate::files::find_char_matches(
                        &doc.text,
                        &doc.tools.find_query,
                        doc.tools.match_case,
                    );
                    match crate::files::replacement_edit(
                        &doc.text,
                        &hits,
                        &doc.tools.replace_text,
                        doc.tools.match_index,
                        matches!(action, GpuiAction::ReplaceEditorAll),
                    ) {
                        Ok(Some((range, replacement))) => {
                            doc.text.replace_range(range, &replacement);
                            doc.unpinned_preview = false;
                            doc.sheet_cache = None;
                            doc.preview = None;
                            doc.last_edited_at = Some(std::time::Instant::now());
                        }
                        Err(error) => doc.error = error,
                        Ok(None) => {}
                    }
                }
            }
            GpuiAction::SetEditorGotoSpec(spec) => {
                if let Some(doc) = self.documents.get_mut(self.active_document) {
                    doc.tools.goto_line = spec;
                }
            }
            GpuiAction::ApplyEditorGoto => {
                if let Some(doc) = self.documents.get_mut(self.active_document)
                    && let Some((offset, line)) =
                        crate::files::goto_line_char_offset(&doc.text, &doc.tools.goto_line)
                {
                    doc.tools.pending_char_range = Some((offset, offset));
                    doc.tools.pending_scroll_line = Some(line);
                    doc.tools.goto_open = false;
                }
            }
            GpuiAction::EditActiveDocumentLine { line_idx, new_line } => {
                if let Some(doc) = self.documents.get_mut(self.active_document)
                    && !doc.media
                {
                    let uses_crlf = doc.text.contains("\r\n");
                    let sep = if uses_crlf { "\r\n" } else { "\n" };
                    let mut lines: Vec<String> = doc
                        .text
                        .split('\n')
                        .map(|s| s.trim_end_matches('\r').to_owned())
                        .collect();
                    if line_idx < lines.len() {
                        lines[line_idx] = new_line;
                        doc.text = lines.join(sep);
                        doc.unpinned_preview = false;
                        doc.sheet_cache = None;
                        doc.preview = None;
                        doc.last_edited_at = Some(std::time::Instant::now());
                    }
                }
            }
            GpuiAction::SelectNativeEditorMatch {
                instance_id,
                base,
                query,
                from_char,
            } => {
                if self.view == View::Files
                    && !self.gpui_has_open_modal()
                    && let Some(doc) = self.documents.get_mut(self.active_document)
                    && doc.instance_id() == instance_id
                    && doc.text == base
                    && doc.tools.find_open
                    && doc.tools.find_query == query
                {
                    let hits =
                        crate::files::find_char_matches(&doc.text, &query, doc.tools.match_case);
                    doc.tools.match_index = hits.iter().position(|h| h.0 >= from_char).unwrap_or(0);
                    doc.tools.pending_char_range = None;
                    doc.tools.pending_scroll_line = None;
                    if let Some(hit) = hits.get(doc.tools.match_index) {
                        doc.tools.pending_char_range = Some((hit.0, hit.1));
                        doc.tools.pending_scroll_line = Some(hit.2);
                    }
                }
            }
            GpuiAction::ApplyNativeEditorToolEdit {
                instance_id,
                field,
                base,
                text,
            } => {
                if text.len() <= crate::files::MAX_TEXT
                    && self.gpui_editor_tool_value(instance_id, field) == Some(base.as_str())
                    && text != base
                {
                    self.apply_gpui_action(match field {
                        EditorToolField::Find => GpuiAction::SetEditorFindQuery(text),
                        EditorToolField::Replace => GpuiAction::SetEditorReplaceText(text),
                        EditorToolField::Goto => GpuiAction::SetEditorGotoSpec(text),
                    });
                }
            }
            GpuiAction::ApplyNativeDocumentEdit {
                instance_id,
                base,
                text,
            } => {
                if self.view == View::Files
                    && !self.gpui_has_open_modal()
                    && let Some(doc) = self.documents.get_mut(self.active_document)
                    && !doc.preview_mode
                {
                    doc.apply_native_edit(instance_id, &base, text);
                }
            }
            GpuiAction::ImageMenu(id, position) => {
                if self.view == View::Files
                    && !self.gpui_has_open_modal()
                    && let Some(doc) = self.documents.get_mut(self.active_document)
                    && doc.instance_id() == id
                    && doc.image.is_some()
                {
                    doc.tools.image_menu = position;
                }
            }
            GpuiAction::DownloadImage(id) => {
                if self.view == View::Files
                    && !self.gpui_has_open_modal()
                    && let Some(doc) = self.documents.get_mut(self.active_document)
                    && doc.instance_id() == id
                    && doc.tools.image_menu.is_some()
                {
                    doc.choose_image_copy(self.state.language == crate::i18n::Language::French);
                }
            }
            GpuiAction::PdfFit(id) => {
                if let Some(pdf) = self
                    .documents
                    .get_mut(self.active_document)
                    .filter(|doc| doc.instance_id() == id)
                    .and_then(|doc| doc.pdf.as_mut())
                {
                    pdf.fit_width();
                }
            }
            GpuiAction::StepPdfZoom(id, delta) => {
                if let Some(pdf) = self
                    .documents
                    .get_mut(self.active_document)
                    .filter(|doc| doc.instance_id() == id)
                    .and_then(|d| d.pdf.as_mut())
                {
                    pdf.step_zoom(delta);
                }
            }
            GpuiAction::SelectOfficeSection(idx) => {
                if let Some(office) = self
                    .documents
                    .get_mut(self.active_document)
                    .and_then(|d| d.office.as_mut())
                {
                    office.select_section(idx);
                }
            }
            GpuiAction::HtmlLinkMenu(id, href, pos) => {
                if self.view == View::Files
                    && !self.gpui_has_open_modal()
                    && let Some(doc) = self.documents.get_mut(self.active_document)
                    && doc.instance_id() == id
                    && doc.preview_mode
                    && let Ok(target) = crate::preview::resolve(&doc.root, &doc.path, &href)
                {
                    doc.tools.html_link_menu =
                        Some((pos, href, matches!(target, crate::preview::Link::Web(_))));
                }
            }
            #[cfg(windows)]
            GpuiAction::AskHtmlTrust(id) => {
                if self.gpui_media_unobscured()
                    && self
                        .active_html_folder()
                        .is_some_and(|(active, _)| active == id)
                {
                    self.editor_page.reconsider = Some(id);
                }
            }
            #[cfg(windows)]
            GpuiAction::DecideHtmlTrust(id, trusted) => {
                if self.gpui_media_unobscured()
                    && let Some((active, folder)) = self.active_html_folder()
                    && active == id
                {
                    if !trusted {
                        self.browser.revoke_preview_scripts(&folder);
                        for page in self.other_browser_pages.values_mut() {
                            page.revoke_preview_scripts(&folder);
                        }
                        for page in self.side_browsers.values_mut() {
                            page.revoke_preview_scripts(&folder);
                        }
                        for (_, page) in &mut self.finishing_browser_pages {
                            page.revoke_preview_scripts(&folder);
                        }
                        for (browser, _) in self.background_browsers.values_mut() {
                            browser.revoke_preview_scripts(&folder);
                        }
                    }
                    if let Err(error) = self.html_trust.decide(folder, trusted) {
                        self.error = error;
                    }
                    self.editor_page.reconsider = None;
                    self.editor_page.set_scripts(trusted);
                }
            }
            #[cfg(windows)]
            GpuiAction::ResolveHtmlExternal(id, shown, open) => {
                let active = match self.view {
                    View::Files => self.documents.get(self.active_document).is_some_and(|d| {
                        d.instance_id() == id && d.preview_mode && d.tools.html_plain_text.is_none()
                    }),
                    View::Remote => self.remote.active_html_page(id).is_some(),
                    _ => false,
                };
                if active
                    && self.gpui_media_unobscured()
                    && let Some(url) = self.editor_page.resolve_external(id, &shown)
                    && open
                {
                    self.open_tessel_url_headless(url.into());
                }
            }
            GpuiAction::CloseHtmlLinkMenu => {}
            GpuiAction::OpenHtmlLinkMenuTarget(id) => {
                if self.view == View::Files
                    && !self.gpui_has_open_modal()
                    && let Some(doc) = self.documents.get_mut(self.active_document)
                    && doc.instance_id() == id
                    && doc.preview_mode
                    && let Some((_, href, _)) = doc.tools.html_link_menu.take()
                {
                    self.apply_gpui_action_inner(GpuiAction::FollowDocumentLink(id, href));
                }
            }
            GpuiAction::FollowDocumentLink(id, href) => {
                if self.view == View::Files
                    && !self.gpui_has_open_modal()
                    && let Some((root, source)) = self
                        .documents
                        .get(self.active_document)
                        .filter(|d| d.instance_id() == id && d.preview_mode)
                        .map(|d| (d.root.clone(), d.path.clone()))
                {
                    match crate::preview::resolve(&root, &source, &href) {
                        Ok(crate::preview::Link::File(target)) => {
                            self.open_linked_document(root, target);
                        }
                        Ok(crate::preview::Link::Web(url)) => {
                            self.open_tessel_url_headless(url);
                        }
                        Err(e) => self.error = e,
                    }
                }
            }
            GpuiAction::BeginNativeBrowserAddress(scope) => {
                #[cfg(windows)]
                if let Some(page) = self.address_page_mut(scope) {
                    page.begin_native_address_edit();
                }
                #[cfg(not(windows))]
                let _ = scope;
            }
            GpuiAction::CancelNativeBrowserAddress(scope) => {
                #[cfg(windows)]
                if let Some(page) = self.address_page_mut(scope) {
                    page.cancel_native_address_edit();
                }
                #[cfg(not(windows))]
                let _ = scope;
            }
            GpuiAction::BrowserAddressCommand { scope, command } => {
                #[cfg(windows)]
                if let Some(page) = self.address_page_mut(scope) {
                    match command {
                        AddressCommand::Back => page.go_back_now(),
                        AddressCommand::Forward => page.go_forward_now(),
                        AddressCommand::Reload => page.reload_now(),
                        AddressCommand::Find => page.open_find(),
                        AddressCommand::HardReload => page.hard_reload(),
                    }
                }
                #[cfg(not(windows))]
                let _ = (scope, command);
            }
            GpuiAction::NativeBrowserAddress {
                scope,
                base,
                text,
                navigate,
            } => {
                #[cfg(windows)]
                let side = self
                    .browser_address_for(true)
                    .is_some_and(|(active, _)| active == scope);
                #[cfg(windows)]
                if text.len() <= 8192
                    && let Some(page) = self.address_page_mut(scope)
                    && page.display_address() == base
                {
                    page.set_native_address_draft(text);
                    if navigate {
                        if side {
                            page.submit_address();
                        } else {
                            page.navigate_now();
                        }
                    }
                }
                #[cfg(not(windows))]
                let _ = (scope, base, text, navigate);
            }
            GpuiAction::BrowserSetAddress(addr) => {
                #[cfg(windows)]
                self.browser.set_address_str(addr);
                #[cfg(not(windows))]
                let _ = addr;
            }
            GpuiAction::BrowserNavigate => {
                #[cfg(windows)]
                self.browser.navigate_now();
            }
            GpuiAction::SideBrowserBack(id)
            | GpuiAction::SideBrowserForward(id)
            | GpuiAction::SideBrowserReload(id) => {
                #[cfg(windows)]
                if self.side_browser_visible(id)
                    && let Some(page) = self.side_browsers.get(&id)
                {
                    match action {
                        GpuiAction::SideBrowserBack(_) => page.go_back_now(),
                        GpuiAction::SideBrowserForward(_) => page.go_forward_now(),
                        _ => page.reload_now(),
                    }
                }
                #[cfg(not(windows))]
                let _ = id;
            }
            GpuiAction::BrowserBack(tab) => {
                #[cfg(windows)]
                if self.browser.tab_id == tab && self.gpui_browser_address().is_some() {
                    self.browser.go_back_now();
                }
                #[cfg(not(windows))]
                let _ = tab;
            }
            GpuiAction::BrowserForward(tab) => {
                #[cfg(windows)]
                if self.browser.tab_id == tab && self.gpui_browser_address().is_some() {
                    self.browser.go_forward_now();
                }
                #[cfg(not(windows))]
                let _ = tab;
            }
            GpuiAction::BrowserTryHttps(tab, url) => {
                #[cfg(windows)]
                if self.browser.tab_id == tab && self.gpui_browser_address().is_some() {
                    self.browser.try_https(&url);
                } else if self.side_browser_visible(tab)
                    && let Some(page) = self.side_browsers.get_mut(&tab)
                {
                    page.try_https(&url);
                }
                #[cfg(not(windows))]
                let _ = (tab, url);
            }
            GpuiAction::BrowserReloadMenu(tab) => {
                #[cfg(windows)]
                if self.browser.tab_id == tab && self.gpui_browser_address().is_some() {
                    self.browser.show_reload_menu();
                } else if self.side_browser_visible(tab)
                    && let Some(page) = self.side_browsers.get(&tab)
                {
                    page.show_reload_menu();
                }
                #[cfg(not(windows))]
                let _ = tab;
            }
            GpuiAction::BrowserReload(tab) => {
                #[cfg(windows)]
                if self.browser.tab_id == tab && self.gpui_browser_address().is_some() {
                    self.browser.reload_now();
                }
                #[cfg(not(windows))]
                let _ = tab;
            }
            GpuiAction::BrowserStop { tab, navigation } => {
                #[cfg(windows)]
                if self.browser.tab_id == tab && self.gpui_browser_address().is_some() {
                    self.browser.stop_navigation(navigation);
                } else if self.side_browser_visible(tab)
                    && let Some(page) = self.side_browsers.get(&tab)
                {
                    page.stop_navigation(navigation);
                }
                #[cfg(not(windows))]
                let _ = (tab, navigation);
            }
            GpuiAction::BrowserImportCookies => {
                #[cfg(windows)]
                self.browser.import_cookie_file_now();
            }
            GpuiAction::BrowserAcceptPopup => {
                #[cfg(windows)]
                self.browser.accept_popup_now();
            }
            GpuiAction::BrowserDismissPopup => {
                #[cfg(windows)]
                self.browser.dismiss_popup_now();
            }
            GpuiAction::BrowserToggleDownloads(tab) => {
                #[cfg(windows)]
                if let Some(page) = self.download_browser_mut(tab) {
                    page.toggle_downloads_open();
                }
                #[cfg(not(windows))]
                let _ = tab;
            }
            GpuiAction::BrowserCancelDownload { tab, id } => {
                #[cfg(windows)]
                if let Some(page) = self.download_browser_mut(tab) {
                    page.cancel_download_now(id);
                }
                #[cfg(not(windows))]
                let _ = (tab, id);
            }
            GpuiAction::BrowserRevealDownload { tab, id } => {
                #[cfg(windows)]
                if let Some(page) = self.download_browser_mut(tab) {
                    page.reveal_download(id);
                }
                #[cfg(not(windows))]
                let _ = (tab, id);
            }
            GpuiAction::Project(pa) => {
                let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                self.apply_project_action(&ctx, Some(pa));
            }
            GpuiAction::SetSidebarPrefs(prefs) => {
                self.state.sidebar = prefs;
                self.save();
            }
            GpuiAction::SidePanels(spa) => {
                if let crate::side_panels_ui::SidePanelsAction::CopyText(ref txt) = spa {
                    self.gpui_extra.copied_clipboard = Some(txt.clone());
                }
                let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                self.handle_side_panels_action(&ctx, spa);
            }
            GpuiAction::SetDashboardGroup(g) => {
                self.side_panels_ui.dashboard_group = g;
            }
            GpuiAction::SetDashboardFilter(f) => {
                self.side_panels_ui.dashboard_filter = f;
            }
            GpuiAction::SetDashboardQuery(q) => {
                self.side_panels_ui.dashboard_query = q;
            }
            GpuiAction::NativePanelQueryEdit { scope, base, text } => {
                if text.len() <= 8192
                    && self.gpui_panel_query_input() == Some((scope, base.as_str()))
                {
                    match scope.target {
                        GpuiFocusTarget::OutlineQuery => {
                            if let Some((_, query)) = &mut self.gpui_extra.outline_query {
                                *query = text;
                            }
                        }
                        GpuiFocusTarget::TaskNewTitle => self.gpui_extra.new_task_title = text,
                        GpuiFocusTarget::DashboardQuery => {
                            self.side_panels_ui.dashboard_query = text
                        }
                        GpuiFocusTarget::TaskHistoryQuery => {
                            self.side_panels_ui.history_query = text
                        }
                        GpuiFocusTarget::SessionsQuery => self.side_panels_ui.sessions_query = text,
                        GpuiFocusTarget::ConversationsQuery => {
                            self.side_panels_ui.conv_query = text
                        }
                        _ => {}
                    }
                }
            }
            GpuiAction::NativePanelQueryExitFullscreen { scope, base } => {
                if scope.target == GpuiFocusTarget::SessionsQuery
                    && self.side_panels_ui.sessions_modal_open
                    && self.gpui_panel_query_input() == Some((scope, base.as_str()))
                {
                    self.side_panels_ui.sessions_modal_open = false;
                }
                if (base.is_empty() || scope.target == GpuiFocusTarget::TaskNewTitle)
                    && scope.target != GpuiFocusTarget::SessionsQuery
                    && self.gpui_panel_query_input() == Some((scope, base.as_str()))
                {
                    self.right_fullscreen = false;
                }
            }
            GpuiAction::NativeExplorerEdit { scope, base, text } => {
                if text.len() <= 8192
                    && self
                        .gpui_explorer_input()
                        .is_some_and(|(s, t)| s == scope && t == base)
                {
                    self.explorer_tree.gpui_set_filter_query(text);
                }
            }
            GpuiAction::ExplorerSetFilter(q) => {
                self.explorer_tree.gpui_set_filter_query(q);
            }
            GpuiAction::ExplorerToggleEditors => {
                self.state.editor_prefs.open_editors_shown =
                    !self.state.editor_prefs.open_editors_shown;
                self.save();
            }
            GpuiAction::ExplorerActivateDocument(id) | GpuiAction::ExplorerCloseDocument(id) => {
                if let Some(i) = self
                    .documents
                    .iter()
                    .position(|d| d.project == self.state.active_project && d.instance_id() == id)
                {
                    if matches!(action, GpuiAction::ExplorerActivateDocument(_)) {
                        self.active_document = i;
                        self.view = View::Files;
                    } else {
                        self.request_document_close(i);
                    }
                }
            }
            GpuiAction::ExplorerSaveAll(project) => {
                if self.state.active_project == Some(project)
                    && !self.file_action_pending
                    && !self.transcript_trash_busy
                {
                    for doc in self.documents.iter_mut().filter(|d| {
                        d.project == Some(project) && d.diff_title.is_none() && d.dirty()
                    }) {
                        match doc.save(self.store.as_ref().map(|s| s.directory())) {
                            Ok(()) => doc.error.clear(),
                            Err(e) => {
                                self.error = e.clone();
                                doc.error = e;
                            }
                        }
                    }
                }
            }
            GpuiAction::ExplorerCloseAll(project) => {
                if self.state.active_project == Some(project) && self.pending_close.is_none() {
                    self.remaining_close_requests = self
                        .documents
                        .iter()
                        .rev()
                        .filter(|d| d.project == Some(project))
                        .map(|d| d.instance_id())
                        .collect();
                    self.advance_close_requests();
                }
            }
            GpuiAction::ExplorerSetSearchMode(content) => {
                self.explorer_tree.gpui_set_search_mode(content);
            }
            GpuiAction::ExplorerPaneHeight(pane, height) => {
                let stored = match pane {
                    ExplorerPane::Outline => &mut self.state.editor_prefs.explorer_outline_height,
                    ExplorerPane::Timeline => &mut self.state.editor_prefs.explorer_timeline_height,
                };
                let height = height.clamp(44, 2000);
                if *stored != height {
                    *stored = height;
                    self.save();
                }
            }
            GpuiAction::ExplorerToggleFolder => {
                self.state.editor_prefs.explorer_folder_shown =
                    !self.state.editor_prefs.explorer_folder_shown;
                self.save();
            }
            GpuiAction::ExplorerCollapseFolders => self.explorer_tree.collapse_all(),
            GpuiAction::ExplorerToggleSearchMode => {
                self.explorer_tree.gpui_toggle_search_mode();
            }
            GpuiAction::ExplorerToggleDotfiles => {
                let next = !self.explorer_tree.show_dotfiles();
                self.explorer_tree.set_show_dotfiles(next);
                self.state.editor_prefs.show_git_ignored = next;
                self.save();
            }
            GpuiAction::Timeline(crate::file_timeline::Action::OpenWeb(url)) => {
                if cfg!(windows)
                    && self.state.browser_prefs.open_links_in
                        != crate::workspace::OpenLinksTarget::System
                {
                    self.open_tessel_url_headless(url);
                } else if let Err(e) = open::that_detached(&url) {
                    self.error = e.to_string();
                }
            }
            GpuiAction::Timeline(action) => self.timeline_action(action),
            GpuiAction::RevealBreadcrumb(id, depth, range) => {
                if let Some(doc) = self.gpui_editor_document()
                    && doc.instance_id() == id
                    && let Some(cache) = self
                        .gpui_extra
                        .outline_cache
                        .as_ref()
                        .filter(|cache| cache.matches(doc))
                    && cache
                        .current_breadcrumbs(doc)
                        .get(depth)
                        .and_then(|c| c.range)
                        == Some(range)
                {
                    let doc = &mut self.documents[self.active_document];
                    crate::json_outline_panel::reveal(doc, range, false);
                    doc.tools.outline_reveal_focus = Some(true);
                }
            }
            GpuiAction::OutlineToggle => {
                self.gpui_extra.outline_open = !self.gpui_extra.outline_open;
                self.gpui_extra.outline_options_open = false;
            }
            GpuiAction::OutlineStartQuery(id, seed) => {
                if self.outline_document_id() == Some(id) {
                    match &mut self.gpui_extra.outline_query {
                        Some((owner, q)) if *owner == id => q.push_str(&seed),
                        query => *query = Some((id, seed)),
                    }
                }
            }
            GpuiAction::OutlineCloseQuery(id) => {
                if self
                    .gpui_extra
                    .outline_query
                    .as_ref()
                    .is_some_and(|(owner, _)| *owner == id)
                {
                    self.gpui_extra.outline_query = None;
                }
            }
            GpuiAction::OutlineFilterMode(on) => {
                self.state.editor_prefs.outline_filter_on_type = on;
                self.gpui_extra.outline_options_open = false;
                self.save();
            }
            GpuiAction::OutlineNavigate(id, key) => {
                if self.outline_document_id() == Some(id)
                    && let Some(index) = self.local_explorer_document_index()
                    && let Some(doc) = self.documents.get_mut(index)
                    && let Some(cache) = self
                        .gpui_extra
                        .outline_cache
                        .as_ref()
                        .filter(|c| c.matches(doc))
                {
                    let query = self
                        .gpui_extra
                        .outline_query
                        .as_ref()
                        .map_or("", |(_, q)| q.trim());
                    let rows = crate::outline_tree::visible_rows(
                        &cache.symbols,
                        &doc.tools.outline_folds,
                        query,
                        !self.state.editor_prefs.outline_filter_on_type,
                    );
                    if key == eframe::egui::Key::Enter
                        && rows.iter().any(|entry| {
                            Some(&entry.row.key) == doc.tools.outline_selected.as_ref()
                        })
                    {
                        self.active_document = index;
                        self.view = View::Files;
                    }
                    if crate::json_outline_panel::navigate(doc, &rows, !query.is_empty(), key) {
                        doc.tools.outline_scroll_generation =
                            doc.tools.outline_scroll_generation.wrapping_add(1);
                    }
                }
            }
            GpuiAction::OutlineOptions(open) => {
                self.gpui_extra.outline_options_open = open;
            }
            GpuiAction::OutlineFollow(on) => {
                self.state.editor_prefs.outline_follow_cursor = on;
                self.gpui_extra.outline_options_open = false;
                self.save();
            }
            GpuiAction::OutlineSort(sort) => {
                self.gpui_extra.outline_options_open = false;
                if self.state.editor_prefs.outline_sort != sort {
                    self.state.editor_prefs.outline_sort = sort;
                    self.gpui_extra.outline_cache = None;
                    self.save();
                }
            }
            GpuiAction::OutlineRow(id, tree, action) => {
                if self.outline_document_id() == Some(id)
                    && let Some(index) = self.local_explorer_document_index()
                    && let Some(doc) = self.documents.get_mut(index)
                    && self.gpui_extra.outline_cache.as_ref().is_some_and(|c| {
                        std::sync::Arc::ptr_eq(&tree, &c.symbols) && c.matches(doc)
                    })
                {
                    match action {
                        OutlineRowAction::SetAllFolds { expand } => {
                            crate::json_outline_panel::set_all_folds(doc, &tree, expand);
                        }
                        OutlineRowAction::Fold(key) => {
                            crate::json_outline_panel::toggle_fold(doc, &key)
                        }
                        OutlineRowAction::Reveal(key, range, select) => {
                            self.active_document = index;
                            self.view = View::Files;
                            doc.tools.outline_selected = Some(key);
                            crate::json_outline_panel::reveal(doc, range, select)
                        }
                    }
                }
            }
            GpuiAction::ExplorerToggleCompact => {
                let next = !self.explorer_tree.compact_folders();
                self.explorer_tree.set_compact_folders(next);
                self.state.editor_prefs.compact_folders = next;
                self.save();
            }
            GpuiAction::ExplorerMenu(scope, key, command) => {
                if self
                    .gpui_explorer_input()
                    .is_none_or(|(live, _)| live != scope)
                {
                    return;
                }
                let root = scope.owner.root;
                let Some((path, directory)) = self.explorer_tree.menu_entry(&root, key.as_deref())
                else {
                    return;
                };
                use ExplorerCommand::*;
                match command {
                    Select => {}
                    Download if key.is_some() => self.download_project_item(root, path),
                    InsertPath if key.is_some() => self.insert_explorer_path(&root, &path, None),
                    TerminalHere => self.open_project_terminal_here(&root, &path),
                    Open if !directory => self.open_document_preview(path),
                    OpenEditor if !directory => self.open_document(path),
                    ExternalEditor if !directory => self.open_external_project_file(root, path),
                    CopyPath => {
                        self.gpui_extra.copied_clipboard =
                            Some(dunce::simplified(&path).display().to_string())
                    }
                    CopyRelative => {
                        self.gpui_extra.copied_clipboard = path
                            .strip_prefix(&root)
                            .ok()
                            .map(|rel| rel.display().to_string())
                    }
                    #[cfg(windows)]
                    Reveal => self.reveal_project_item(root, path),
                    Dotfiles => self.apply_gpui_action_inner(GpuiAction::ExplorerToggleDotfiles),
                    Compact => self.apply_gpui_action_inner(GpuiAction::ExplorerToggleCompact),
                    NewFile | NewFolder | Rename | Trash if self.file_actions_ready() => {
                        use crate::file_actions::{Action, Kind};
                        let (kind, path, name) = match command {
                            NewFile | NewFolder => {
                                let dest = if directory {
                                    path
                                } else {
                                    let Some(p) = path.parent() else { return };
                                    p.to_owned()
                                };
                                (
                                    if command == NewFile {
                                        Kind::File
                                    } else {
                                        Kind::Folder
                                    },
                                    dest,
                                    String::new(),
                                )
                            }
                            Rename | Trash if key.is_some() => {
                                let Some(name) =
                                    path.file_name().map(|n| n.to_string_lossy().into_owned())
                                else {
                                    return;
                                };
                                (
                                    if command == Rename {
                                        Kind::Rename
                                    } else {
                                        Kind::Trash
                                    },
                                    path,
                                    name,
                                )
                            }
                            _ => return,
                        };
                        self.file_action = Some(Action {
                            kind,
                            root,
                            path,
                            name,
                        });
                    }
                    _ => {}
                }
            }
            GpuiAction::ExplorerRefresh => {
                self.explorer_tree.refresh();
            }
            GpuiAction::ExplorerNavigate(root, key) => {
                if !self.state.editor_prefs.explorer_folder_shown
                    || self
                        .gpui_explorer_owner()
                        .is_none_or(|owner| owner.root != root)
                {
                    return;
                }
                use crate::explorer_tree::{TreeAction, TreeContextKind};
                match self.explorer_tree.navigate_tree(key) {
                    Some(TreeAction::OpenFile(path)) => self.open_document_preview(path),
                    Some(TreeAction::Context {
                        kind,
                        root: action_root,
                        path,
                        name,
                    }) if action_root == root && self.file_actions_ready() => {
                        let kind = match kind {
                            TreeContextKind::Rename => crate::file_actions::Kind::Rename,
                            TreeContextKind::Trash => crate::file_actions::Kind::Trash,
                            _ => return,
                        };
                        self.file_action = Some(crate::file_actions::Action {
                            kind,
                            root,
                            path,
                            name,
                        });
                    }
                    _ => {}
                }
            }
            GpuiAction::ExplorerClickRow { key, pinned } => {
                if let Some(act) = self.explorer_tree.gpui_click_row(&key, pinned) {
                    match act {
                        crate::explorer_tree::TreeAction::OpenFile(p) => {
                            self.open_document_preview(p);
                        }
                        crate::explorer_tree::TreeAction::OpenPinnedFile(p) => {
                            self.open_document(p);
                        }
                        crate::explorer_tree::TreeAction::CopyPath(p) => {
                            self.gpui_extra.copied_clipboard = Some(p.display().to_string());
                        }
                        _ => {}
                    }
                }
            }
            GpuiAction::ExplorerOpenContentHit { path, line } => {
                self.open_explorer_content_hit(path, line)
            }
            GpuiAction::ExplorerNewItem { dir } => {
                if let Some(root) = self.state.active().map(|p| p.path.clone()) {
                    if !self.state.editor_prefs.explorer_folder_shown {
                        self.state.editor_prefs.explorer_folder_shown = true;
                        self.save();
                    }
                    self.file_action = Some(crate::file_actions::Action {
                        kind: if dir {
                            crate::file_actions::Kind::Folder
                        } else {
                            crate::file_actions::Kind::File
                        },
                        path: root.clone(),
                        root,
                        name: String::new(),
                    });
                }
            }
            GpuiAction::ExplorerDeletePath { path, permanent } => {
                if let Some(root) = self.state.active().map(|p| p.path.clone()) {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    self.file_action = Some(crate::file_actions::Action {
                        kind: if permanent {
                            crate::file_actions::Kind::Delete
                        } else {
                            crate::file_actions::Kind::Trash
                        },
                        root,
                        path,
                        name,
                    });
                }
            }
            GpuiAction::ScmRefresh => {
                self.sync_git_project();
                self.scm_state.refresh();
                self.refresh_scm_commits();
            }
            GpuiAction::NativeScmEdit { scope, base, text } => {
                if text.len() <= 65536
                    && self.gpui_scm_input() == Some((scope.clone(), base.as_str()))
                {
                    self.gpui_extra.scm_drafts.set(&scope.owner, text);
                }
            }
            GpuiAction::NativeScmTouch { scope, base } => {
                if self.gpui_scm_input() == Some((scope.clone(), base.as_str())) {
                    self.gpui_extra.scm_drafts.touch(&scope.owner, scope.id);
                }
            }
            GpuiAction::NativeScmSubmit { scope, base } => {
                if self.gpui_scm_input() == Some((scope, base.as_str())) && base.len() <= 65536 {
                    self.apply_gpui_action(GpuiAction::ScmCommit);
                }
            }
            GpuiAction::ScmSetCommitMessage(msg) => {
                if let Some(owner) = self.gpui_scm_owner() {
                    self.gpui_extra.scm_drafts.set(&owner, msg);
                }
            }
            GpuiAction::ScmCommit => {
                self.sync_git_project();
                let draft = self
                    .gpui_scm_draft()
                    .map(|(o, _, t)| (o, t.trim().to_owned()));
                if let Some((owner, message)) = draft {
                    if message.is_empty() {
                        self.error = tr("Enter a commit message.").into();
                    } else if !self.git_result.busy() {
                        if let Err(err) = self.scm_state.validate_commit() {
                            self.error = err;
                        } else {
                            self.git_start_work(move |root| {
                                crate::git::commit_staged(&root, &message)
                            });
                            if self.git_result.pending_owner() == Some(&owner) {
                                self.commit_in_flight = true;
                                self.gpui_extra.scm_drafts.submitted(&owner);
                            }
                        }
                    }
                } else {
                    self.error = tr("Select a project first.").into();
                }
            }
            GpuiAction::ScmStage(paths) => {
                if !self.git_result.busy() {
                    match self.scm_state.stage_paths(&paths) {
                        Ok(valid) => {
                            self.git_start_work(move |root| crate::git::stage_files(&root, &valid));
                        }
                        Err(e) => self.error = e,
                    }
                }
            }
            GpuiAction::ScmUnstage(paths) => {
                if !self.git_result.busy() {
                    match self.scm_state.unstage_paths(&paths) {
                        Ok(valid) => {
                            self.git_start_work(move |root| {
                                crate::git::unstage_files(&root, &valid)
                            });
                        }
                        Err(e) => self.error = e,
                    }
                }
            }
            GpuiAction::ScmRequestDiscard { area, paths } => {
                if !self.git_result.busy() && !paths.is_empty() {
                    let french = self.state.language == crate::i18n::Language::French;
                    let copy = crate::scm_changes_panel::discard_area_copy(
                        area,
                        paths.len(),
                        false,
                        french,
                    );
                    self.pending_scm_discard = Some((area, paths, copy));
                }
            }
            GpuiAction::ScmOpenDiff {
                area,
                path,
                old_path,
                pinned,
            } => {
                self.open_scm_diff(area, path, old_path, pinned);
            }
            GpuiAction::ScmRemoteOp(op) => {
                if !self.git_result.busy() {
                    let branch = self.scm_state.branch_name();
                    self.git_start_work(move |root| {
                        crate::git::remote_op(&root, op, branch.as_deref())
                    });
                }
            }
            GpuiAction::ScmToggleCommitsOpen => {
                self.side_panels_ui.commits_open = !self.side_panels_ui.commits_open;
                if self.side_panels_ui.commits_open {
                    self.refresh_scm_commits();
                }
            }
            GpuiAction::OpenNotesModal => {
                self.open_notes_modal();
            }
            GpuiAction::SetNotesText(text) => {
                self.side_panels_ui.notes.text = text;
            }
            GpuiAction::OpenReviewModal => {
                self.open_review_modal();
            }
            GpuiAction::TasksSetNewTitle(t) => {
                self.gpui_extra.new_task_title = t;
            }
            GpuiAction::NativeTaskSubmit { scope, base } => {
                if scope.target == GpuiFocusTarget::TaskNewTitle
                    && base.chars().count() <= crate::tasks_board_panel::MAX_TASK_TITLE_CHARS
                    && self.gpui_panel_query_input() == Some((scope, base.as_str()))
                {
                    self.apply_gpui_action(GpuiAction::TasksAddFromDraft);
                }
            }
            GpuiAction::TasksAddFromDraft => {
                let title = self.gpui_extra.new_task_title.trim().to_owned();
                if !title.is_empty()
                    && let Some(pid) = self.state.active_project
                {
                    let added = self.apply_tasks_board_action(
                        pid,
                        crate::tasks_board_panel::TasksBoardAction::Add(title),
                    );
                    if added {
                        self.gpui_extra.new_task_title.clear();
                        self.gpui_extra.panel_query_generation = self
                            .gpui_extra
                            .panel_query_generation
                            .checked_add(1)
                            .expect("panel identity exhausted");
                    }
                }
            }
            GpuiAction::NativeTaskBoard { scope, command } => {
                if (!self.gpui_has_open_modal()
                    || matches!(
                        command,
                        crate::native_task_board::Command::ConfirmDelete
                            | crate::native_task_board::Command::CancelDelete
                    ))
                    && let Some(action) =
                        self.gpui_extra
                            .task_board
                            .apply(scope, command, &self.state.tasks)
                {
                    self.apply_tasks_board_action(scope.project, action);
                }
            }
            GpuiAction::TasksBoard(tba) => {
                if let Some(pid) = self.state.active_project {
                    self.apply_tasks_board_action(pid, tba);
                }
            }
            GpuiAction::SetTaskHistoryWindow(w) => {
                self.side_panels_ui.history_window = w;
            }
            GpuiAction::SetTaskHistorySort(s) => {
                self.side_panels_ui.history_sort = s;
            }
            GpuiAction::ToggleTaskHistoryCurrentProject => {
                self.side_panels_ui.history_current_project_only =
                    !self.side_panels_ui.history_current_project_only;
            }
            GpuiAction::SetTaskHistoryQuery(q) => {
                self.side_panels_ui.history_query = q;
            }
            GpuiAction::SetSessionsScope(sc) => {
                self.side_panels_ui.sessions_scope = sc;
            }
            GpuiAction::SetSessionsGroup(g) => {
                self.side_panels_ui.sessions_group = g;
            }
            GpuiAction::ToggleSessionsTranscripts => {
                self.side_panels_ui.sessions_include_transcripts =
                    !self.side_panels_ui.sessions_include_transcripts;
            }
            GpuiAction::SetSessionsQuery(q) => {
                self.side_panels_ui.sessions_query = q;
            }
            GpuiAction::SetConversationsWindow(w) => {
                self.side_panels_ui.conv_window = w;
            }
            GpuiAction::SetConversationsQuery(q) => {
                self.side_panels_ui.conv_query = q;
            }
            GpuiAction::SelectConversationThread(k) => {
                self.side_panels_ui.selected_thread_key = k;
            }
            GpuiAction::SetSettingsSearch(q) => {
                self.gpui_extra.settings_search = q;
            }
            GpuiAction::SettingsPage(spa) => {
                let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                self.handle_settings_page_action(&ctx, spa);
            }
            GpuiAction::SettingsExtra(sexa) => {
                let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                self.handle_settings_extra_action(&ctx, sexa);
            }
            GpuiAction::UpdateGeneralPrefs(p) => {
                self.state.general_prefs = p;
                self.save();
            }
            GpuiAction::UpdateTerminalPrefs(p) => {
                self.state.terminal_prefs = p;
                self.save();
            }
            GpuiAction::SetEditorFontSize { base, size } => {
                if u32::from(self.state.editor_prefs.font_size) == base
                    && (6..=100).contains(&size)
                    && size != base
                {
                    self.state.editor_prefs.font_size = size as u8;
                    self.save();
                }
            }
            GpuiAction::SetAutoSaveDelay { base, delay } => {
                let prefs = &mut self.state.editor_prefs;
                if prefs.auto_save
                    && prefs.auto_save_delay_ms == base
                    && delay != base
                    && (250..=10000).contains(&delay)
                {
                    prefs.auto_save_delay_ms = delay;
                    self.save();
                }
            }
            GpuiAction::UpdateEditorPrefs(p) => {
                self.explorer_tree.set_show_dotfiles(p.show_git_ignored);
                self.explorer_tree.set_compact_folders(p.compact_folders);
                self.state.editor_prefs = p;
                self.save();
            }
            GpuiAction::UpdateBrowserPrefs(p) => {
                self.state.browser_prefs = p;
                self.save();
            }
            GpuiAction::UpdateGitPrefs(p) => {
                self.scm_panel
                    .set_group_order(p.scm_group_order.into_panel());
                self.scm_panel.set_view_mode(p.scm_view_mode.into_panel());
                self.state.git_prefs = p;
                self.save();
            }
            GpuiAction::UpdateStatsPrefs(p) => {
                self.state.stats_prefs = p;
                self.save();
            }
            GpuiAction::UpdateContextPrefs(p) => {
                self.state.context_prefs = p;
                self.save();
                self.refresh_context_scan();
            }
            GpuiAction::UpdateOrchestrationPrefs(p) => {
                self.state.orchestration_prefs = p;
                self.save();
            }
            GpuiAction::UpdateNotificationPrefs(p) => {
                self.state.notification_prefs = p;
                self.save();
            }
            GpuiAction::UpdateVoicePrefs(p) => {
                self.state.voice_prefs = p;
                self.save();
            }
            GpuiAction::ToggleBuiltinAgentEnabled(id) => {
                let cur = crate::agent_preferences::enabled(&self.state, &id);
                let canonical = crate::agent_catalog::find(&id).map_or(id.as_str(), |e| e.id);
                self.state
                    .agent_prefs
                    .entry(canonical.to_owned())
                    .or_default()
                    .enabled = !cur;
                self.save();
            }
            GpuiAction::SetQuickCommandDraft {
                name,
                text,
                press_enter,
            } => {
                self.gpui_extra.new_quick_cmd_name = name;
                self.gpui_extra.new_quick_cmd_text = text;
                self.gpui_extra.new_quick_cmd_enter = press_enter;
            }
            GpuiAction::SubmitNewQuickCommand => {
                let name = self.gpui_extra.new_quick_cmd_name.trim().to_owned();
                let text = self.gpui_extra.new_quick_cmd_text.trim().to_owned();
                let press_enter = self.gpui_extra.new_quick_cmd_enter;
                if !name.is_empty() && !text.is_empty() {
                    self.gpui_extra.new_quick_cmd_name.clear();
                    self.gpui_extra.new_quick_cmd_text.clear();
                    let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                    self.handle_settings_page_action(
                        &ctx,
                        crate::settings_pages_panel::SettingsPageAction::AddQuickCommand {
                            name,
                            text,
                            press_enter,
                        },
                    );
                }
            }
            GpuiAction::SetSshDraft { alias, folder } => {
                self.gpui_extra.new_ssh_alias = alias;
                self.gpui_extra.new_ssh_folder = folder;
            }
            GpuiAction::ImportSshConfig => self.import_ssh_config(),
            GpuiAction::BeginSshEdit(index, expected) => {
                if self.state.hosts.get(index) == Some(&expected) {
                    self.gpui_extra.new_ssh_alias.clone_from(&expected.alias);
                    self.gpui_extra.new_ssh_folder.clone_from(&expected.folder);
                    self.gpui_extra.editing_ssh = Some((index, expected));
                    self.host_notice = None;
                } else {
                    self.host_notice = Some((
                        tr("This SSH host changed; reopen its settings.").into(),
                        true,
                    ));
                }
            }
            GpuiAction::CancelSshEdit => {
                self.gpui_extra.editing_ssh = None;
                self.gpui_extra.new_ssh_alias.clear();
                self.gpui_extra.new_ssh_folder.clear();
                self.host_notice = None;
            }
            GpuiAction::SaveSshHostFromDraft => {
                let original = self.gpui_extra.editing_ssh.clone();
                let expected = original.as_ref().map(|(index, host)| (*index, host));
                let host = crate::workspace::Host {
                    alias: self.gpui_extra.new_ssh_alias.trim().to_owned(),
                    folder: self.gpui_extra.new_ssh_folder.trim().to_owned(),
                    ..expected.map(|(_, host)| host.clone()).unwrap_or_default()
                };
                match self.change_hosts(|hosts| crate::host_actions::save(hosts, expected, host)) {
                    Ok(_) => {
                        self.apply_gpui_action(GpuiAction::CancelSshEdit);
                        self.host_notice = Some((tr("SSH target saved").into(), false));
                    }
                    Err(error) => self.host_notice = Some((tr(&error).into(), true)),
                }
            }
            GpuiAction::RemoveSshHost(index, expected) => {
                match self.remove_saved_host(index, &expected) {
                    Ok(_) => self.host_notice = Some((tr("SSH target removed").into(), false)),
                    Err(error) => self.host_notice = Some((tr(&error).into(), true)),
                }
            }
            GpuiAction::SshRow(index, expected, command) => {
                if self.state.hosts.get(index) != Some(&expected) {
                    self.host_notice = Some((
                        tr("This SSH host changed; reopen its settings.").into(),
                        true,
                    ));
                    return;
                }
                match command {
                    SshRowCommand::Test => self.test_ssh_host(expected),
                    SshRowCommand::Disconnect => {
                        self.disconnect_host_sessions(&expected);
                        self.host_notice = Some((tr("SSH connections closed").into(), false));
                    }
                    SshRowCommand::OpenTerminal => {
                        self.overlay = None;
                        self.new_session_placed(Launch::ssh(expected), None);
                    }
                    SshRowCommand::TogglePersistence | SshRowCommand::ToggleDaemon => {
                        let mut updated = expected.clone();
                        if matches!(command, SshRowCommand::TogglePersistence) {
                            updated.persistent_sessions =
                                expected.persistent_sessions_enabled().then_some(false);
                        } else {
                            updated.host_daemon = !expected.host_daemon;
                        }
                        self.host_notice = Some(
                            match self.change_hosts(|hosts| {
                                crate::host_actions::save(hosts, Some((index, &expected)), updated)
                            }) {
                                Ok(_) => (tr("SSH target saved").into(), false),
                                Err(error) => (tr(&error).into(), true),
                            },
                        );
                    }
                }
            }
            GpuiAction::ToggleAgentFolderTrust => {
                self.state.agent_folder_trust = !self.state.agent_folder_trust;
                self.save();
            }
            GpuiAction::SetCustomAgentDraft { name, cmd } => {
                self.gpui_extra.new_custom_agent_name = name;
                self.gpui_extra.new_custom_agent_cmd = cmd;
            }
            GpuiAction::SetCustomAgentAccent(index) => {
                if index < crate::settings_extra_model::CUSTOM_AGENT_SWATCHES.len() {
                    self.gpui_extra.new_custom_agent_accent = index;
                }
            }
            GpuiAction::SubmitNewCustomAgent => {
                let name = self.gpui_extra.new_custom_agent_name.trim();
                let command = self.gpui_extra.new_custom_agent_cmd.trim();
                if name.is_empty() || command.is_empty() {
                    self.settings_extra.custom_error =
                        "Enter a name and the command that starts the agent.".into();
                    return;
                }
                let result = crate::settings_extra_model::generate_custom_agent_id(
                    &self.state.custom_agents,
                )
                .map(|id| crate::settings_extra_model::CustomAgent {
                    id,
                    name: name.into(),
                    command: command.into(),
                    accent: crate::settings_extra_model::CUSTOM_AGENT_SWATCHES
                        [self.gpui_extra.new_custom_agent_accent]
                        .into(),
                    enabled: true,
                })
                .and_then(|agent| self.save_custom_agent(agent));
                match result {
                    Ok(()) => {
                        self.gpui_extra.new_custom_agent_name.clear();
                        self.gpui_extra.new_custom_agent_cmd.clear();
                        self.settings_extra.custom_error.clear();
                    }
                    Err(error) => self.settings_extra.custom_error = error,
                }
            }
            GpuiAction::SetAccountLabelDraft(lbl) => {
                self.gpui_extra.new_account_label = lbl;
            }
            GpuiAction::SelectSecretKeyIndex(idx) => {
                self.gpui_extra.secret_key_idx = idx.min(
                    crate::settings_extra_model::PROVIDER_SECRET_KEYS
                        .len()
                        .saturating_sub(1),
                );
            }
            GpuiAction::SetSecretValueDraft(val) => {
                self.gpui_extra.secret_value_draft = val;
            }
            GpuiAction::SaveSecretFromDraft => {
                let idx = self.gpui_extra.secret_key_idx.min(
                    crate::settings_extra_model::PROVIDER_SECRET_KEYS
                        .len()
                        .saturating_sub(1),
                );
                let key = crate::settings_extra_model::PROVIDER_SECRET_KEYS[idx].to_owned();
                let val = self.gpui_extra.secret_value_draft.trim().to_owned();
                if !val.is_empty() {
                    self.gpui_extra.secret_value_draft.clear();
                    let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                    self.handle_settings_extra_action(
                        &ctx,
                        crate::settings_extra_panel::SettingsExtraAction::SaveProviderSecret {
                            key,
                            value: val,
                        },
                    );
                }
            }
            GpuiAction::SetSkillsGitUrlDraft(url) => {
                self.gpui_extra.skills_git_url = url;
            }
            GpuiAction::SetAutomationDraft {
                name,
                prompt,
                every_mins,
            } => {
                self.gpui_extra.new_auto_name = name;
                self.gpui_extra.new_auto_prompt = prompt;
                self.gpui_extra.new_auto_every_mins = every_mins.max(15);
            }
            GpuiAction::SubmitNewAutomation => {
                let name = self.gpui_extra.new_auto_name.trim().to_owned();
                let prompt = self.gpui_extra.new_auto_prompt.trim().to_owned();
                let mins = self.gpui_extra.new_auto_every_mins.max(15);
                if !name.is_empty()
                    && !prompt.is_empty()
                    && let Some(proj) = self.state.active().cloned()
                {
                    let schedule = if mins >= 60 && mins.is_multiple_of(60) {
                        crate::automations::AutomationSchedule::Hourly { minute: 0 }
                    } else {
                        crate::automations::AutomationSchedule::Cron {
                            expr: format!("*/{} * * * *", mins.min(59)),
                        }
                    };
                    let id = match crate::automations::generate_automation_id("auto") {
                        Ok(id) => id,
                        Err(error) => {
                            self.error = error;
                            return;
                        }
                    };
                    if self
                        .automations_store
                        .automations
                        .iter()
                        .any(|auto| auto.id == id)
                    {
                        self.error =
                            "Automation identifier already exists; retry creating the automation."
                                .into();
                        return;
                    }
                    let auto = crate::automations::Automation {
                        id,
                        name,
                        project_id: proj.id,
                        project_path: proj.path,
                        agent: crate::automations::AutomationAgent::Claude,
                        isolation: crate::automations::AutomationIsolation::Worktree,
                        schedule,
                        prompt,
                        enabled: false,
                        confirmed_sig: None,
                        last_minute_slot: None,
                        runs: Vec::new(),
                    };
                    self.gpui_extra.new_auto_name.clear();
                    self.gpui_extra.new_auto_prompt.clear();
                    let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                    self.handle_settings_extra_action(
                        &ctx,
                        crate::settings_extra_panel::SettingsExtraAction::UpsertAutomation(auto),
                    );
                }
            }
            GpuiAction::VaultUnlock => {
                #[cfg(windows)]
                self.vault.gpui_unlock();
            }
            GpuiAction::VaultImportCsv => {
                #[cfg(windows)]
                self.vault.gpui_import_csv();
            }
            GpuiAction::VaultDeleteEntry(id) => {
                #[cfg(windows)]
                self.vault.gpui_delete_entry(id);
                #[cfg(not(windows))]
                let _ = id;
            }
            GpuiAction::VaultFillEntry(id) => {
                #[cfg(windows)]
                self.vault.gpui_fill_entry(id, &self.browser);
                #[cfg(not(windows))]
                let _ = id;
            }
            GpuiAction::AccountOpenStore => {
                #[cfg(windows)]
                self.account.gpui_open_store();
            }
            GpuiAction::AccountSignInBrowser => {
                #[cfg(windows)]
                self.account.gpui_begin_browser_sign_in();
            }
            GpuiAction::AccountRefresh => {
                #[cfg(windows)]
                self.account.gpui_refresh();
            }
            GpuiAction::AccountOpenAdminPortal => {
                #[cfg(windows)]
                let _ = webbrowser::open(crate::account::ADMIN_PORTAL_URL);
            }
            GpuiAction::AccountSignOut => {
                #[cfg(windows)]
                self.account.gpui_sign_out();
            }
            GpuiAction::AccountSuggestPhrase => {
                #[cfg(windows)]
                self.account.gpui_suggest_sync_phrase();
            }
            GpuiAction::ToggleExpandBuiltinAgent(id) => {
                if self.gpui_extra.expanded_agent_id.as_deref() == Some(id.as_str()) {
                    self.gpui_extra.expanded_agent_id = None;
                } else {
                    self.gpui_extra.expanded_agent_id = Some(id);
                }
            }
            GpuiAction::RefreshBuiltinAgentModels(id) => {
                let command = crate::agent_preferences::get(&self.state, &id)
                    .map(|p| p.command.clone())
                    .unwrap_or_default();
                if let Some(discovery) = &mut self.model_discovery {
                    discovery.refresh(&id, &command, || {});
                }
            }
            GpuiAction::ResetBuiltinAgentPrefs(id) => {
                let mut prefs = crate::agent_preferences::get(&self.state, &id)
                    .cloned()
                    .unwrap_or_default();
                prefs.reset();
                self.save_builtin_agent_prefs(&id, prefs);
            }
            GpuiAction::DismissMcpCardMessage(key) => {
                self.settings_extra.mcp_card_messages.remove(&key);
            }
            GpuiAction::ToggleDevUpdateDialog => {
                self.dev_update_dialog_open = !self.dev_update_dialog_open;
                if self.dev_update_dialog_open {
                    self.dev_update_notice = None;
                }
            }
            GpuiAction::RefreshDevUpdate => {
                self.dev_update_notice = None;
                self.dev_update.refresh_now(true);
            }
            GpuiAction::ApplyDevUpdate => {
                self.dev_update_notice = None;
                self.dev_update.apply_now();
            }
            GpuiAction::CopyGitPullCommand => {
                self.gpui_extra.copied_clipboard = Some("git pull --ff-only".into());
                self.dev_update_notice =
                    Some(if self.state.language == crate::i18n::Language::French {
                        "Commande « git pull --ff-only » copiée.".into()
                    } else {
                        "Copied \"git pull --ff-only\".".into()
                    });
            }
            GpuiAction::NativeSearchEdit { scope, base, text } => {
                if self.gpui_native_search_input() == Some((scope, base.as_str()))
                    && (scope.kind != NativeSearchKind::Finder
                        || text.chars().take(257).count() <= 256)
                {
                    match scope.kind {
                        NativeSearchKind::Palette => self.command_palette.set_query(text),
                        NativeSearchKind::Finder => self.finder.gpui_set_query(text),
                    }
                }
            }
            GpuiAction::NativeSearchStep { scope, delta } => {
                if self.gpui_native_search_input().map(|v| v.0) == Some(scope) {
                    self.apply_gpui_action(match scope.kind {
                        NativeSearchKind::Palette => GpuiAction::PaletteStep(delta),
                        NativeSearchKind::Finder => GpuiAction::FinderStep(delta),
                    });
                }
            }
            GpuiAction::NativeSearchAccept {
                scope,
                query,
                choice,
                insert,
            } => {
                if self.gpui_native_search_input() != Some((scope, query.as_str())) {
                    return;
                }
                match scope.kind {
                    NativeSearchKind::Palette => {
                        if let Some(id) = choice {
                            let commands = self.build_palette_commands(
                                self.state.theme.unwrap_or(ThemeId::Classic),
                            );
                            if crate::palette::filter_palette_commands(&commands, &query)
                                .iter()
                                .any(|r| r.command.id == id)
                            {
                                self.apply_gpui_action(GpuiAction::PaletteExecute(id));
                            }
                        } else {
                            self.apply_gpui_action(GpuiAction::PaletteExecuteSelected);
                        }
                    }
                    NativeSearchKind::Finder => {
                        let idx = if let Some(relative) = choice {
                            let Some(snapshot) = self.finder.gpui_snapshot() else {
                                return;
                            };
                            let Some(index) = snapshot.results.iter().position(|r| *r == relative)
                            else {
                                return;
                            };
                            Some(index)
                        } else {
                            None
                        };
                        self.apply_gpui_action(GpuiAction::FinderAccept { idx, insert });
                    }
                }
            }
            GpuiAction::ToggleCommandPalette => self.command_palette.toggle(),
            GpuiAction::PaletteSetQuery(q) => {
                self.command_palette.query = q;
                self.command_palette.index = 0;
            }
            GpuiAction::PaletteStep(delta) => {
                let current_theme = self.state.theme.unwrap_or(ThemeId::Classic);
                let cmds = self.build_palette_commands(current_theme);
                let rows =
                    crate::palette::filter_palette_commands(&cmds, &self.command_palette.query);
                if !rows.is_empty() {
                    let len = rows.len() as isize;
                    let cur = (self.command_palette.index.min(rows.len() - 1)) as isize;
                    self.command_palette.index = (cur + delta).rem_euclid(len) as usize;
                }
            }
            GpuiAction::PaletteExecute(cmd_id) => {
                self.command_palette.open = false;
                let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                self.execute_palette_command(&ctx, &cmd_id);
            }
            GpuiAction::PaletteExecuteSelected => {
                let current_theme = self.state.theme.unwrap_or(ThemeId::Classic);
                let cmds = self.build_palette_commands(current_theme);
                let rows =
                    crate::palette::filter_palette_commands(&cmds, &self.command_palette.query);
                let picked = rows
                    .get(self.command_palette.index)
                    .or_else(|| rows.first())
                    .map(|r| r.command.id.clone());
                self.command_palette.open = false;
                if let Some(cmd_id) = picked {
                    let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                    self.execute_palette_command(&ctx, &cmd_id);
                }
            }
            GpuiAction::OpenFinder => {
                self.open_finder();
            }
            GpuiAction::FinderSetQuery(q) => {
                self.finder.gpui_set_query(q);
            }
            GpuiAction::FinderStep(delta) => {
                self.finder.gpui_step(delta);
            }
            GpuiAction::FinderAccept { idx, insert } => {
                let act = match idx {
                    Some(i) => self.finder.gpui_accept_index(i, insert),
                    None => self.finder.gpui_accept(insert),
                };
                if let Some(action) = act {
                    let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                    self.handle_finder_action(&ctx, action);
                }
            }
            GpuiAction::ToggleLaunchMenu => {
                self.gpui_extra.launch_menu_open = !self.gpui_extra.launch_menu_open;
                if !self.gpui_extra.launch_menu_open && self.overlay == Some(Overlay::Launcher) {
                    self.overlay = None;
                }
            }
            GpuiAction::SetLaunchPlacement(p) => {
                self.gpui_extra.launch_placement = p;
            }
            GpuiAction::ToggleLaunchInWorktree => {
                self.gpui_extra.launch_in_worktree = !self.gpui_extra.launch_in_worktree;
            }
            GpuiAction::LaunchFromMenu {
                agent_id,
                native_chat,
            } => {
                self.gpui_extra.launch_menu_open = false;
                if self.overlay == Some(Overlay::Launcher) {
                    self.overlay = None;
                }
                let placement = match (self.gpui_extra.launch_placement, self.active_session) {
                    (crate::finder_model::LaunchPlacement::Left, Some(sid))
                    | (crate::finder_model::LaunchPlacement::Right, Some(sid)) => {
                        Some((sid, SplitDir::Row))
                    }
                    (crate::finder_model::LaunchPlacement::Down, Some(sid)) => {
                        Some((sid, SplitDir::Col))
                    }
                    _ => None,
                };
                if let Some(nc) = native_chat {
                    if nc == "codex" {
                        self.apply_gpui_action(GpuiAction::LaunchCodexChat);
                    } else {
                        self.apply_gpui_action(GpuiAction::LaunchClaudeChat);
                    }
                } else if let Some(aid) = agent_id {
                    self.new_session_placed(Launch::Agent(aid), placement);
                } else {
                    self.new_session_placed(Launch::Shell, placement);
                }
            }
            GpuiAction::OpenSettings => {
                if self.overlay == Some(Overlay::Settings) {
                    self.overlay = None;
                } else {
                    self.overlay = Some(Overlay::Settings);
                }
            }
            GpuiAction::OpenSettingsPage(page_id) => {
                self.gpui_extra.settings_search.clear();
                self.open_settings_by_page_id(&page_id);
            }
            GpuiAction::CloseOverlays => {
                if self.new_task.open {
                    self.new_task_command(
                        self.new_task.epoch,
                        crate::new_task_form::Command::Cancel,
                    );
                    return;
                }
                if self.linear.view.open {
                    self.linear.command(
                        self.linear.view.epoch,
                        crate::linear_model::panel::Command::Close,
                    );
                    return;
                }
                if let Some(scope) = self.gpui_extra.task_board.scope {
                    self.gpui_extra.task_board.apply(
                        scope,
                        crate::native_task_board::Command::CancelDelete,
                        &self.state.tasks,
                    );
                }
                self.command_palette.open = false;
                self.finder.close();
                self.gpui_extra.launch_menu_open = false;
                self.gpui_extra.usage_popover_open = false;
                self.gpui_extra.layout_menu_right = None;
                self.gpui_extra.toolbar_more_open = false;
                self.gpui_extra.issues_menu_open = false;
                self.gpui_extra.notifications_popover_open = false;
                self.gpui_extra.status_popover = None;
                self.overlay = None;
                self.dialogs_toolbar_ui.add_project_open = false;
                self.dialogs_toolbar_ui.github_open = false;
                self.dialogs_toolbar_ui.group_modal_project = None;
                self.side_panels_ui.notes_modal_open = false;
                self.side_panels_ui.review_modal_open = false;
                self.side_panels_ui.sessions_modal_open = false;
                self.worktree_cleanup = None;
                self.teams_ui.close_manager();
                self.share.close_pill_popover();
                self.rename_project_modal = None;
                self.rename_agent = None;
                self.remaining_close_requests.clear();
                self.pending_close = None;
                self.pending_close_session = None;
                self.pending_remove_project = None;
                self.pending_scm_discard = None;
                self.pending_delete_worktree = None;
                self.pending_agent_install = None;
                self.pending_transcript_trash = None;
                self.file_action = None;
            }
            GpuiAction::OpenHelp => {
                if self.overlay == Some(Overlay::Help) {
                    self.overlay = None;
                } else {
                    self.overlay = Some(Overlay::Help);
                }
            }
            GpuiAction::NativeModalQuery { scope, base, text } => {
                if text.len() <= 8192 && self.gpui_modal_query() == Some((scope, base.as_str())) {
                    match scope.1 {
                        GpuiFocusTarget::HelpQuery => self.gpui_extra.help_query = text,
                        GpuiFocusTarget::SettingsSearch => self.gpui_extra.settings_search = text,
                        GpuiFocusTarget::GuestTaskDraft(guest_scope) => {
                            self.share.edit_guest_task_draft(guest_scope, text);
                        }
                        _ => {}
                    }
                }
            }
            GpuiAction::DismissNativeModalQuery { scope, base } => {
                if self.gpui_modal_query() == Some((scope, base.as_str())) {
                    self.apply_gpui_action(GpuiAction::CloseOverlays);
                }
            }
            GpuiAction::SetHelpQuery(q) => {
                self.gpui_extra.help_query = q;
            }
            GpuiAction::OpenLiveShare => {
                if self.overlay == Some(Overlay::Share) {
                    self.overlay = None;
                } else {
                    self.overlay = Some(Overlay::Share);
                }
            }
            GpuiAction::Share(sa) => {
                let all_projects = self.live_share_project_list();
                self.share.apply_gpui_share_action(sa, &all_projects);
            }
            GpuiAction::OpenLinearDialog => self.open_linear_modal(),
            GpuiAction::OpenGitHubDialog => {
                self.open_github_modal(self.state.active_project);
            }
            GpuiAction::DialogsToolbar(dta) => {
                let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                self.handle_dialogs_toolbar_action(&ctx, dta);
            }
            GpuiAction::SetAddProjectTab(tab) => {
                self.dialogs_toolbar_ui.add_tab = tab;
            }
            GpuiAction::ToggleNestedRepo(path) => {
                if !self.dialogs_toolbar_ui.selected_nested_repos.remove(&path) {
                    self.dialogs_toolbar_ui.selected_nested_repos.insert(path);
                }
            }
            GpuiAction::SetAddProjectCloneUrl(url) => {
                let default_name = crate::dialogs_toolbar_model::default_repo_folder_name(&url);
                if self.dialogs_toolbar_ui.clone_folder_name.is_empty() {
                    self.dialogs_toolbar_ui.clone_folder_name = default_name;
                }
                self.dialogs_toolbar_ui.clone_url = url;
            }
            GpuiAction::SetAddProjectFolderName(name) => match self.dialogs_toolbar_ui.add_tab {
                crate::dialogs_toolbar_ui::AddProjectTab::CloneRepo => {
                    self.dialogs_toolbar_ui.clone_folder_name = name;
                }
                crate::dialogs_toolbar_ui::AddProjectTab::SshRemote => {
                    self.dialogs_toolbar_ui.ssh_remote_dir = name;
                }
                _ => {
                    self.dialogs_toolbar_ui.init_folder_name = name;
                }
            },
            GpuiAction::SetAddProjectSshAlias(alias) => {
                self.dialogs_toolbar_ui.ssh_selected_alias = alias;
            }
            GpuiAction::SetGitHubTab(tab) => {
                self.dialogs_toolbar_ui.github_tab = tab;
            }
            GpuiAction::SetGitHubQuery(q) => {
                self.dialogs_toolbar_ui.github_query = q;
            }
            GpuiAction::OpenTeamsManager(pid) => {
                self.teams_ui.open_manager(pid);
            }
            GpuiAction::CloseTeamsManager => {
                self.teams_ui.close_manager();
            }
            GpuiAction::SetNewTeamName(name) => {
                self.teams_ui.new_team_name = name;
            }
            GpuiAction::Teams(tua) => {
                self.handle_teams_ui_action(tua);
            }
            GpuiAction::WorktreeCleanupTogglePath(p) => {
                if let Some(modal) = &mut self.worktree_cleanup
                    && !modal.selected.remove(&p)
                {
                    modal.selected.insert(p);
                }
            }
            GpuiAction::WorktreeCleanupSelectSafe => {
                if let Some(modal) = &mut self.worktree_cleanup {
                    modal.select_safe();
                }
            }
            GpuiAction::WorktreeCleanupSelectNone => {
                if let Some(modal) = &mut self.worktree_cleanup {
                    modal.selected.clear();
                }
            }
            GpuiAction::WorktreeCleanupConfirmRemove => {
                if let Some(modal) = self.worktree_cleanup.take() {
                    let pid = modal.project_id;
                    let batch: Vec<(PathBuf, String, bool)> = modal
                        .items
                        .into_iter()
                        .filter(|it| modal.selected.contains(&it.path) && it.blocker.is_none())
                        .map(|it| {
                            let force = !it.risks.is_empty();
                            (it.path, it.branch, force)
                        })
                        .collect();
                    if !batch.is_empty()
                        && let Some(proj) =
                            self.state.projects.iter().find(|p| p.id == pid).cloned()
                    {
                        let paths: Vec<PathBuf> = batch.iter().map(|(p, _, _)| p.clone()).collect();
                        self.close_sessions_for_worktree_paths(pid, &paths);
                        self.worktree_state
                            .request_remove_batch(pid, proj.path, batch);
                    }
                }
            }
            GpuiAction::WorktreeCleanupClose => {
                self.worktree_cleanup = None;
            }
            GpuiAction::ToggleUsagePopover => {
                self.gpui_extra.usage_popover_open = !self.gpui_extra.usage_popover_open;
                if self.gpui_extra.usage_popover_open {
                    self.gpui_extra.notifications_popover_open = false;
                }
            }
            GpuiAction::ToggleNotificationsPopover => {
                self.gpui_extra.notifications_popover_open =
                    !self.gpui_extra.notifications_popover_open;
                if self.gpui_extra.notifications_popover_open {
                    self.gpui_extra.usage_popover_open = false;
                }
            }
            GpuiAction::ToggleStatusPopover(item) => {
                if self.gpui_extra.status_popover == Some(item) {
                    self.gpui_extra.status_popover = None;
                } else {
                    self.gpui_extra.status_popover = Some(item);
                }
            }
            GpuiAction::ToggleIssuesMenu => {
                self.gpui_extra.issues_menu_open = !self.gpui_extra.issues_menu_open;
                self.gpui_extra.toolbar_more_open = false;
                self.gpui_extra.usage_popover_open = false;
                self.gpui_extra.notifications_popover_open = false;
            }
            GpuiAction::ToggleToolbarMore => {
                self.gpui_extra.issues_menu_open = false;
                self.gpui_extra.toolbar_more_open = !self.gpui_extra.toolbar_more_open;
                if self.gpui_extra.toolbar_more_open {
                    self.gpui_extra.usage_popover_open = false;
                    self.gpui_extra.notifications_popover_open = false;
                }
            }
            GpuiAction::ToolbarMore(item) => {
                self.gpui_extra.issues_menu_open = false;
                self.gpui_extra.toolbar_more_open = false;
                self.apply_toolbar_more_action(item);
            }
            GpuiAction::RefreshPortsNow => {
                self.status_telemetry.refresh_ports_now();
            }
            GpuiAction::SetPromptDraft(draft) => {
                if let Some((_, d, _)) = &mut self.rename_project_modal {
                    *d = draft;
                } else if let Some(ra) = &mut self.rename_agent {
                    ra.draft = draft;
                } else if self.dialogs_toolbar_ui.group_modal_project.is_some() {
                    self.dialogs_toolbar_ui.group_modal_draft = draft;
                } else if let Some(fa) = &mut self.file_action {
                    fa.name = draft;
                }
            }
            GpuiAction::SubmitPromptModal => {
                if let Some((pid, draft, _)) = self.rename_project_modal.take() {
                    let trimmed = draft.trim();
                    if !trimmed.is_empty()
                        && let Some(p) = self.state.projects.iter_mut().find(|p| p.id == pid)
                    {
                        p.name = trimmed.to_owned();
                        self.save();
                    }
                } else if let Some(ra) = self.rename_agent.take() {
                    let trimmed = ra.draft.trim();
                    if let Some(sess) = self.sessions.iter_mut().find(|s| s.id == ra.pane) {
                        sess.pane_name = (!trimmed.is_empty()).then(|| trimmed.to_owned());
                        self.save();
                    }
                } else if let Some(pid) = self.dialogs_toolbar_ui.group_modal_project.take() {
                    let grp = self.dialogs_toolbar_ui.group_modal_draft.trim().to_owned();
                    let ctx = self.gpui_extra.dummy_egui_ctx.clone();
                    self.handle_dialogs_toolbar_action(
                        &ctx,
                        crate::dialogs_toolbar_ui::DialogsToolbarAction::SaveProjectGroup {
                            project_id: pid,
                            group: (!grp.is_empty()).then_some(grp),
                        },
                    );
                } else if let Some(fa) = self.file_action.take() {
                    self.file_action_pending = true;
                    let sender = self.sender.clone();
                    thread::spawn(move || {
                        let _ = sender
                            .send(super::Message::FileAction(crate::file_actions::execute(fa)));
                    });
                }
            }
            GpuiAction::CancelPromptModal => {
                self.rename_project_modal = None;
                self.rename_agent = None;
                self.dialogs_toolbar_ui.group_modal_project = None;
                self.file_action = None;
            }
            GpuiAction::ConfirmModalPrimary => {
                if let Some(idx) = self.pending_close {
                    if let Some(doc) = self.documents.get_mut(idx) {
                        match doc.save(self.store.as_ref().map(|s| s.directory())) {
                            Ok(()) => {
                                self.remove_document(idx);
                                self.advance_close_requests();
                            }
                            Err(e) => self.error = e,
                        }
                    }
                } else if let Some(sid) = self.pending_close_session.take() {
                    self.close_session_immediate(sid);
                } else if let Some(pid) = self.pending_remove_project.take() {
                    self.remove_project_immediate(pid);
                } else if let Some((area, paths, _)) = self.pending_scm_discard.take() {
                    match self.scm_state.discard_selection(area, &paths) {
                        Ok((tracked, untracked)) => {
                            self.git_start_work(move |root| {
                                crate::git::discard_worktree(&root, &tracked, &untracked)
                            });
                        }
                        Err(e) => self.error = e,
                    }
                } else if let Some((pid, wt_path, branch)) = self.pending_delete_worktree.take() {
                    if let Some(proj) = self.state.projects.iter().find(|p| p.id == pid).cloned() {
                        self.close_sessions_for_worktree_paths(pid, std::slice::from_ref(&wt_path));
                        self.worktree_state.request_remove_batch(
                            pid,
                            proj.path,
                            vec![(wt_path, branch, true)],
                        );
                    }
                } else if self.gpui_extra.task_board.pending_delete.is_some() {
                    if let Some(scope) = self.gpui_extra.task_board.scope {
                        self.apply_gpui_action(GpuiAction::NativeTaskBoard {
                            scope,
                            command: crate::native_task_board::Command::ConfirmDelete,
                        });
                    }
                } else if self.pending_transcript_trash.is_some() {
                    self.confirm_transcript_trash();
                } else if let Some(spec) = self.pending_agent_install.take() {
                    self.start_agent_install(spec.execution);
                } else if let Some(fa) = self.file_action.take() {
                    self.file_action_pending = true;
                    let sender = self.sender.clone();
                    thread::spawn(move || {
                        let _ = sender
                            .send(super::Message::FileAction(crate::file_actions::execute(fa)));
                    });
                }
            }
            GpuiAction::ConfirmModalSecondary => {
                if let Some(idx) = self.pending_close {
                    self.remove_document(idx);
                    self.advance_close_requests();
                }
            }
            GpuiAction::CancelConfirmModal => {
                if let Some(scope) = self.gpui_extra.task_board.scope {
                    self.gpui_extra.task_board.apply(
                        scope,
                        crate::native_task_board::Command::CancelDelete,
                        &self.state.tasks,
                    );
                }
                self.remaining_close_requests.clear();
                self.pending_close = None;
                self.pending_close_session = None;
                self.pending_remove_project = None;
                self.pending_scm_discard = None;
                self.pending_delete_worktree = None;
                self.pending_agent_install = None;
                self.pending_transcript_trash = None;
                self.file_action = None;
            }
            GpuiAction::SetFileIcons(style) => {
                if self.state.file_icons != style {
                    self.state.file_icons = style;
                    self.save();
                }
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
                    self.refresh_language();
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

#[cfg(test)]
mod panel_snapshot_tests {
    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn explorer_insert_path_pastes_relative_text_without_submitting_in_the_active_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(root));
        let owner = app.state.active().unwrap().path.clone();
        let (id, input) = app.install_terminal_find_fixture();
        assert_eq!(app.explorer_terminal_id(), Some(id));
        app.insert_explorer_path(&owner, &owner.join("docs & notes").join("report.txt"), None);
        assert_eq!(input.try_recv().unwrap(), br#""docs & notes\report.txt""#);
        app.insert_explorer_path(&owner, &owner.join("plain.txt"), None);
        assert_eq!(input.try_recv().unwrap(), b"plain.txt");
        app.insert_explorer_path(&owner, &owner.join("../outside"), None);
        app.insert_explorer_path(&owner, &owner, None);
        app.view = View::Files;
        assert!(app.explorer_terminal_id().is_none());
        app.insert_explorer_path(&owner, &owner.join("plain.txt"), None);
        app.view = View::Work;
        app.active_session = None;
        app.insert_explorer_path(&owner, &owner.join("plain.txt"), None);
        assert!(input.try_recv().is_err());
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn manual_grid_survives_real_launch_save_and_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(project));
        app.gpui_snapshot(1200., 600., 4.);
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        let grid = app.gpui_snapshot(1200., 600., 4.);
        let divider = &grid.dividers[0];
        app.apply_gpui_action(GpuiAction::ResizeSplitDivider {
            split_id: divider.split_id.clone(),
            dir: divider.dir,
            index: divider.index,
            split_side_px: divider.split_side_px,
            delta_px: 180.,
        });
        let original = app.gpui_snapshot(1200., 600., 4.);
        let smaller = &original.tiles[1];
        assert!(original.tiles[0].rect_px.width > smaller.rect_px.width);
        drop(app);
        let mut app = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        app.gpui_snapshot(1200., 600., 4.);
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        let after = app.gpui_snapshot(1200., 600., 4.);
        assert_eq!(after.tiles.len(), 3);
        assert_eq!(
            after
                .tiles
                .iter()
                .find(|tile| tile.session_id == smaller.session_id)
                .unwrap()
                .rect_px,
            smaller.rect_px
        );
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn terminal_tab_of_empty_browser_checkout_restores_launcher_and_starts_chat_there() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        let checkout = dir.path().join("feature");
        std::fs::create_dir(&project).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .current_dir(&project)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q"]);
        git(&[
            "-c",
            "user.name=Tessel Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgSign=false",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ]);
        git(&[
            "worktree",
            "add",
            "-qb",
            "feature",
            checkout.to_str().unwrap(),
        ]);
        let start = || Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let mut app = start();
        app.apply_gpui_action(GpuiAction::AddProjectPath(project));
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        let main = app.active_session.unwrap();
        app.new_browser_page();
        app.new_session_in_worktree(
            app.state.active_project,
            Launch::ClaudeChat,
            None,
            false,
            Some(crate::workspace::SavedWorktree {
                path: checkout.clone(),
                branch: "feature".into(),
                base_branch: None,
            }),
        );
        let old = app.active_session.unwrap();
        app.new_browser_page();
        app.remove_session(old);
        app.apply_gpui_action(GpuiAction::SelectWorkTab);
        let check = |app: &mut Tessel| {
            let snap = app.gpui_snapshot(1200., 640., 4.);
            assert_eq!(snap.center_mode, GpuiCenterMode::EmptyProjectLauncher);
            assert!(snap.tiles.is_empty());
            assert!(snap.view_tabs[0].selected);
            assert_eq!(
                snap.view_tabs
                    .iter()
                    .filter(|t| t.browser_id.is_some())
                    .count(),
                1
            );
            assert!(app.active_session.is_none());
        };
        check(&mut app);
        app.select_browser_page(app.browser.tab_id);
        assert_eq!(
            app.gpui_snapshot(1200., 640., 4.).center_mode,
            GpuiCenterMode::BrowserTab
        );
        app.select_work_tab();
        drop(app);
        let mut app = start();
        check(&mut app);
        app.apply_gpui_action(GpuiAction::LaunchCodexChat);
        assert!(app.pending_browser_launch.is_some());
        app.apply_gpui_action(GpuiAction::FocusSession(main));
        app.poll_worktrees();
        assert!(app.pending_browser_launch.is_none());
        assert_eq!(app.sessions.len(), 1);
        app.select_browser_page(app.browser.tab_id);
        app.select_work_tab();
        app.apply_gpui_action(GpuiAction::LaunchCodexChat);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while app.sessions.len() == 1 {
            app.poll_worktrees();
            assert!(
                std::time::Instant::now() < until,
                "checkout launch timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let session = app.sessions.last().unwrap();
        assert_eq!(
            session.cwd.canonicalize().unwrap(),
            checkout.canonicalize().unwrap()
        );
        assert_eq!(session.worktree.as_ref().unwrap().branch, "feature");
        assert_ne!(app.active_session, Some(main));
        assert_eq!(
            app.gpui_snapshot(1200., 640., 4.).center_mode,
            GpuiCenterMode::WorkGrid
        );
        assert_eq!(app.sessions.len(), 2);
        app.remove_session(app.active_session.unwrap());
        check(&mut app);
        app.apply_gpui_action(GpuiAction::FocusSession(main));
        assert_eq!(
            app.gpui_snapshot(1200., 640., 4.).center_mode,
            GpuiCenterMode::WorkGrid
        );
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn opening_and_closing_panes_preserves_each_checkouts_layout_mode_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        let path = dir.path().join("feature");
        for p in [&main, &path] {
            std::fs::create_dir(p).unwrap();
        }
        let start = || Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let mut app = start();
        app.apply_gpui_action(GpuiAction::AddProjectPath(main));
        let feature = crate::workspace::SavedWorktree {
            path,
            branch: "feature".into(),
            base_branch: None,
        };
        let add = |app: &mut Tessel, wt| {
            app.new_session_in_worktree(
                app.state.active_project,
                Launch::ClaudeChat,
                None,
                false,
                wt,
            );
            app.active_session.unwrap()
        };
        let bounds = |app: &mut Tessel, id| {
            app.apply_gpui_action(GpuiAction::FocusSession(id));
            app.gpui_snapshot(1200., 640., 4.)
                .tiles
                .into_iter()
                .map(|tile| (tile.session_id, tile.rect_px))
                .collect::<Vec<_>>()
        };
        let left = add(&mut app, None);
        let right = add(&mut app, None);
        let first = add(&mut app, Some(feature.clone()));
        let second = add(&mut app, Some(feature.clone()));
        app.arrange_visible_grid();
        app.apply_gpui_action(GpuiAction::FocusSession(left));
        let grid = app.gpui_snapshot(1200., 640., 4.);
        let div = &grid.dividers[0];
        app.apply_gpui_action(GpuiAction::ResizeSplitDivider {
            split_id: div.split_id.clone(),
            dir: div.dir,
            index: div.index,
            split_side_px: div.split_side_px,
            delta_px: 120.,
        });
        let original = bounds(&mut app, right);
        assert!(original[0].1.width > original[1].1.width);
        drop(app);
        let mut app = start();
        let third = add(&mut app, Some(feature.clone()));
        assert_eq!(bounds(&mut app, first).len(), 3);
        let unchanged = bounds(&mut app, left);
        for ((id, a), (other, b)) in original.iter().zip(&unchanged) {
            assert_eq!(id, other);
            assert!((a.width - b.width).abs() < 1e-6);
        }
        app.remove_session(third);
        let pair = bounds(&mut app, second);
        assert_eq!(pair.len(), 2);
        assert!((pair[0].1.width - pair[1].1.width).abs() < 1e-6);
        let new = add(&mut app, None);
        let reshaped = bounds(&mut app, new);
        assert_eq!(reshaped.len(), 3);
        let small = reshaped.iter().find(|(id, _)| *id == right).unwrap().1;
        assert!((small.width - original[1].1.width).abs() < 1e-6);
        assert_eq!(bounds(&mut app, first), pair);
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn arrange_targets_visible_checkout_but_fixed_presets_use_project_root() {
        use crate::layout_broadcast_ui::LayoutMenuAction;
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("main");
        let checkout = dir.path().join("feature");
        for path in [&project, &checkout] {
            std::fs::create_dir(path).unwrap();
        }
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(project.clone()));
        let [main, a, b] = [0; 3].map(|_| {
            app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
            app.active_session.unwrap()
        });
        for id in [a, b] {
            app.sessions
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap()
                .worktree = Some(crate::workspace::SavedWorktree {
                path: checkout.clone(),
                branch: "feature".into(),
                base_branch: None,
            });
        }
        let key = crate::terminal_layout::project_key(app.state.active_project);
        app.state
            .terminal_layouts
            .get_mut(&key)
            .unwrap()
            .rebuild_ordered(&[main, a, b], &[3]);
        let grid = app.gpui_snapshot(1200., 640., 4.);
        assert!(grid.toolbar.can_arrange_panes);
        let div = &grid.dividers[0];
        app.apply_gpui_action(GpuiAction::ResizeSplitDivider {
            split_id: div.split_id.clone(),
            dir: div.dir,
            index: div.index,
            split_side_px: div.split_side_px,
            delta_px: 125.,
        });
        app.new_browser_page();
        app.apply_gpui_action(GpuiAction::LayoutMenu(Some(720.)));
        app.apply_gpui_action(GpuiAction::ChooseLayout(LayoutMenuAction::ArrangeGrid));
        assert!(app.view == View::Browser);
        let grid = app.gpui_snapshot(1200., 640., 4.);
        assert_eq!(grid.tiles.len(), 2);
        assert!((grid.tiles[0].rect_px.width - grid.tiles[1].rect_px.width).abs() < 0.001);
        app.apply_gpui_action(GpuiAction::FocusSession(main));
        assert!(!app.gpui_snapshot(1200., 640., 4.).toolbar.can_arrange_panes);
        let before = app.state.terminal_layouts[&key].clone();
        app.apply_gpui_action(GpuiAction::LayoutMenu(Some(720.)));
        app.apply_gpui_action(GpuiAction::ChooseLayout(LayoutMenuAction::ArrangeGrid));
        assert_eq!(before, app.state.terminal_layouts[&key]);
        app.apply_gpui_action(GpuiAction::FocusSession(b));
        app.new_browser_page();
        app.apply_gpui_action(GpuiAction::LayoutMenu(Some(720.)));
        app.apply_gpui_action(GpuiAction::ChooseLayout(LayoutMenuAction::ApplyPreset(
            crate::pane_grid::GridPreset::TwoByTwo,
        )));
        assert!(app.view == View::Work);
        assert_eq!(app.active_session, Some(main));
        assert_eq!(app.browser_labels().len(), 2);
        assert_eq!(app.sessions.len(), 4);
        let shell = app.sessions.last().unwrap();
        assert!(shell.worktree.is_none());
        assert_eq!(
            shell.cwd.canonicalize().unwrap(),
            project.canonicalize().unwrap()
        );
        assert_eq!(app.gpui_snapshot(1200., 640., 4.).tiles.len(), 2);
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn worktree_grid_resize_focus_and_restore_preserve_hidden_pane_share() {
        let dir = tempfile::tempdir().unwrap();
        let start = || Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let mut app = start();
        app.apply_gpui_action(GpuiAction::AddProjectPath(dir.path().to_owned()));
        let [first, main, last] = [0; 3].map(|_| {
            app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
            app.active_session.unwrap()
        });
        for id in [first, last] {
            app.sessions
                .iter_mut()
                .find(|s| s.id == id)
                .unwrap()
                .worktree = Some(crate::workspace::SavedWorktree {
                path: dir.path().join("feature"),
                branch: "feature".into(),
                base_branch: None,
            });
        }
        let key = crate::terminal_layout::project_key(app.state.active_project);
        app.state
            .terminal_layouts
            .get_mut(&key)
            .unwrap()
            .rebuild_ordered(&[first, main, last], &[3]);
        let main_width = |app: &Tessel| {
            app.state.terminal_layouts[&key]
                .view_geometry(None)
                .tiles
                .into_iter()
                .find(|t| t.leaf.id == main.to_string())
                .unwrap()
                .rect
                .to_px(1200., 640., 4.)
                .width
        };
        let initial = main_width(&app);
        let snap = app.gpui_snapshot(1200., 640., 4.);
        assert_eq!(
            snap.tiles.iter().map(|t| t.session_id).collect::<Vec<_>>(),
            [first, last]
        );
        assert_eq!(app.gpui_neighbor(FocusDir::Left), Some(first));
        let div = &snap.dividers[0];
        app.apply_gpui_action(GpuiAction::ResizeSplitDivider {
            split_id: div.split_id.clone(),
            dir: div.dir,
            index: div.index,
            split_side_px: div.split_side_px,
            delta_px: 100.,
        });
        let updated = app.gpui_snapshot(1200., 640., 4.);
        assert!(updated.tiles[0].rect_px.width > snap.tiles[0].rect_px.width + 90.);
        assert!((main_width(&app) - initial).abs() < 0.001);
        app.apply_gpui_action(GpuiAction::FocusSession(main));
        let root = app.gpui_snapshot(1200., 640., 4.);
        assert_eq!(root.tiles.len(), 1);
        assert_eq!(root.tiles[0].session_id, main);
        assert_eq!(root.tiles[0].rect_px.width, 1200.);
        assert!(root.dividers.is_empty());
        assert!(app.gpui_neighbor(FocusDir::Left).is_none());
        drop(app);
        let mut app = start();
        assert_eq!(app.gpui_snapshot(1200., 640., 4.).tiles.len(), 1);
        app.apply_gpui_action(GpuiAction::FocusSession(last));
        let restored = app.gpui_snapshot(1200., 640., 4.);
        assert_eq!(restored.tiles.len(), 2);
        for (saved, loaded) in updated.tiles.iter().zip(&restored.tiles) {
            assert_eq!(saved.session_id, loaded.session_id);
            let a = saved.rect_px;
            let b = loaded.rect_px;
            for (x, y) in [
                (a.left, b.left),
                (a.top, b.top),
                (a.width, b.width),
                (a.height, b.height),
            ] {
                assert!((x - y).abs() < 1e-6, "restored bounds {a:?} and {b:?}");
            }
        }
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn browser_drop_tracks_target_worktree_across_tab_switch_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        let checkout = dir.path().join("feature");
        for path in [&project, &checkout] {
            std::fs::create_dir(path).unwrap();
        }
        let start = || Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let mut app = start();
        app.apply_gpui_action(GpuiAction::AddProjectPath(project));
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        let main = app.active_session.unwrap();
        app.new_browser_page();
        let main_page = app.browser.tab_id;
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        let feature = app.active_session.unwrap();
        app.sessions.last_mut().unwrap().worktree = Some(crate::workspace::SavedWorktree {
            path: checkout.clone(),
            branch: "feature".into(),
            base_branch: None,
        });
        app.apply_gpui_action(GpuiAction::FocusSession(main));
        app.new_side_browser();
        let page = app.selected_side_browser.unwrap();
        app.apply_gpui_action(GpuiAction::UndockBrowserOnPane {
            page,
            pane: u64::MAX,
        });
        assert_eq!(app.selected_side_browser, Some(page));
        app.apply_gpui_action(GpuiAction::UndockBrowserOnPane {
            page,
            pane: feature,
        });
        let moved = app.browser.tab_id;
        assert_eq!(app.browser.view_worktree.as_ref(), Some(&checkout));
        assert_eq!(
            app.browser_tab_labels(true)
                .iter()
                .map(|t| t.0)
                .collect::<Vec<_>>(),
            [moved]
        );
        assert_eq!(app.browser_labels().len(), 2);
        app.select_browser_page(main_page);
        app.apply_gpui_action(GpuiAction::SelectWorkTab);
        assert_eq!(app.active_session, Some(main));
        app.select_browser_page(moved);
        app.save();
        drop(app);
        let mut app = start();
        assert_eq!(app.browser.view_worktree.as_ref(), Some(&checkout));
        assert_eq!(app.browser_tab_labels(true).len(), 1);
        app.apply_gpui_action(GpuiAction::SplitActiveShell(SplitDir::Row));
        let shell = app.active_session.unwrap();
        assert_eq!(app.sessions.last().unwrap().cwd, checkout);
        assert_eq!(
            app.sessions
                .last()
                .unwrap()
                .worktree
                .as_ref()
                .unwrap()
                .branch,
            "feature"
        );
        let restored = app.browser.tab_id;
        app.select_browser_page(restored);
        app.close_browser_page(restored);
        assert!(app.view == View::Work);
        assert_eq!(app.active_session, Some(shell));
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn split_shell_inherits_its_worktree_and_rejects_a_hidden_target() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let project = dir.path().join("project");
        let checkout = dir.path().join("feature");
        let other = dir.path().join("other");
        for path in [&project, &checkout, &other] {
            std::fs::create_dir(path).unwrap();
        }
        app.apply_gpui_action(GpuiAction::AddProjectPath(project));
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        let source = app.active_session.unwrap();
        let worktree = crate::workspace::SavedWorktree {
            path: checkout.clone(),
            branch: "feature".into(),
            base_branch: Some("main".into()),
        };
        app.sessions.last_mut().unwrap().worktree = Some(worktree.clone());
        app.apply_gpui_action(GpuiAction::NewShellSplit(source, SplitDir::Col));
        let shell = app.sessions.last().unwrap();
        assert_ne!(shell.id, source);
        assert_eq!(shell.cwd, checkout);
        assert_eq!(shell.worktree.as_ref(), Some(&worktree));
        let tiles = app.gpui_snapshot(1000., 700., 4.).tiles;
        assert_eq!(tiles[0].rect_px.left, tiles[1].rect_px.left);
        assert!(tiles[1].rect_px.top > tiles[0].rect_px.top);
        app.apply_gpui_action(GpuiAction::AddProjectPath(other));
        app.apply_gpui_action(GpuiAction::NewShellSplit(source, SplitDir::Col));
        assert_eq!(app.sessions.len(), 2);
        assert!(app.active_project_work_pane().is_none());
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn workspace_focus_returns_to_each_selected_pane_and_restores_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let mut app = Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        let mut projects = Vec::new();
        for name in ["first", "second"] {
            let path = dir.path().join(name);
            std::fs::create_dir(&path).unwrap();
            app.apply_gpui_action(GpuiAction::AddProjectPath(path));
            app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
            let first = app.active_session.unwrap();
            let second = app.install_terminal_find_fixture().0;
            projects.push((app.state.active_project.unwrap(), first, second));
        }
        let [(a, a1, a2), (b, b1, b2)] = projects[..] else {
            unreachable!()
        };
        app.apply_gpui_action(GpuiAction::SelectProject(a));
        assert_eq!(app.active_session, Some(a2), "returning to project A");
        app.apply_gpui_action(GpuiAction::FocusSession(a1));
        app.apply_gpui_action(GpuiAction::CycleWorkspace(1));
        assert_eq!(app.active_session, Some(b2), "cycling to project B");
        app.apply_gpui_action(GpuiAction::FocusSession(b1));
        assert!(app.error.is_empty(), "save failed: {}", app.error);
        drop(app);
        let mut app = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        assert_eq!(
            app.active_session,
            Some(b1),
            "restoring project B: {}",
            app.error
        );
        assert!(app.sessions.iter().all(|s| s.terminal.is_none()));
        app.apply_gpui_action(GpuiAction::SelectProject(a));
        assert_eq!(app.active_session, Some(a1));
        app.remove_session(a1);
        app.apply_gpui_action(GpuiAction::SelectProject(b));
        app.apply_gpui_action(GpuiAction::SelectProject(a));
        assert_eq!(app.active_session, Some(a2));
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn agent_request_reveals_its_project_and_maximized_pane() {
        use crate::dialogs_toolbar_ui::DialogsToolbarAction;

        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let mut app = Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        for name in ["agent", "other"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        app.apply_gpui_action(GpuiAction::AddProjectPath(dir.path().join("agent")));
        let project = app.state.active_project;
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        let agent = app.active_session.unwrap();
        let shell = app.install_terminal_find_fixture().0;
        app.apply_gpui_action(GpuiAction::ToggleMaximizeSession(shell));
        let request = || GpuiAction::DialogsToolbar(DialogsToolbarAction::FocusPane(agent));
        app.apply_gpui_action(request());
        let snapshot = app.gpui_snapshot(900., 600., 4.);
        assert_eq!(snapshot.tiles.len(), 1);
        assert_eq!(app.active_session, Some(agent));
        assert!(snapshot.tiles[0].native_chat.is_some());
        app.apply_gpui_action(GpuiAction::AddProjectPath(dir.path().join("other")));
        let other_shell = app.install_terminal_find_fixture().0;
        app.apply_gpui_action(GpuiAction::ToggleMaximizeSession(other_shell));
        app.notifications.push_coalesced(
            crate::dialogs_toolbar_model::NotificationKind::ApprovalNeeded,
            "Request",
            "",
            Some(agent),
        );
        app.notifications.push_coalesced(
            crate::dialogs_toolbar_model::NotificationKind::SystemNotice,
            "Earlier message",
            "",
            Some(agent),
        );
        app.notifications.push_coalesced(
            crate::dialogs_toolbar_model::NotificationKind::SystemNotice,
            "Other pane",
            "",
            Some(other_shell),
        );
        app.gpui_extra.notifications_popover_open = true;
        let notice = app.notifications.items[0].id;
        for (unread, expected) in [(false, 2), (true, 3)] {
            app.apply_gpui_action(GpuiAction::DialogsToolbar(
                DialogsToolbarAction::SetNotificationUnread(notice, unread),
            ));
            assert_eq!(app.active_session, Some(other_shell));
            assert!(app.gpui_extra.notifications_popover_open);
            assert_eq!(app.notifications.unread_count(), expected);
        }
        app.apply_gpui_action(GpuiAction::DialogsToolbar(
            DialogsToolbarAction::OpenNotification(notice),
        ));
        assert!(!app.gpui_extra.notifications_popover_open);
        assert_eq!(app.notifications.unread_count(), 1);
        assert!(app.notifications.items.last().unwrap().unread);
        assert_eq!(app.gpui_snapshot(900., 600., 4.).tiles.len(), 1);
        assert_eq!(app.state.active_project, project);
        assert_eq!(app.active_session, Some(agent));
        drop(app);
        let app = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        assert_eq!(app.state.active_project, project);
        assert_eq!(app.active_session, Some(agent));
        assert_eq!(app.notifications.items.len(), 3);
        assert_eq!(app.notifications.unread_count(), 1);
        assert_eq!(
            app.notifications.items.last().unwrap().pane_id,
            Some(other_shell)
        );
    }

    #[test]
    fn notification_history_restores_read_status_ids_and_clear_after_restart() {
        use crate::dialogs_toolbar_model::NotificationKind;
        use crate::dialogs_toolbar_ui::DialogsToolbarAction;
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("state");
        let open = || Tessel::new_for_gpui_in_dir(Some(dir.clone()), false, || {});
        let mut app = open();
        app.push_notification(NotificationKind::SystemNotice, "First", "", None);
        app.push_notification(NotificationKind::ApprovalNeeded, "Second", "", None);
        let first = app.notifications.items[0].id;
        app.apply_gpui_action(GpuiAction::DialogsToolbar(
            DialogsToolbarAction::OpenNotification(first),
        ));
        drop(app);
        let mut app = open();
        assert_eq!(app.notifications.items.len(), 2);
        assert_eq!(app.notifications.items[0].id, first);
        assert!(!app.notifications.items[0].unread);
        assert!(app.notifications.items[1].unread);
        let second = app.notifications.items[1].id;
        app.apply_gpui_action(GpuiAction::DialogsToolbar(
            DialogsToolbarAction::SetNotificationUnread(first, true),
        ));
        drop(app);
        let mut app = open();
        assert_eq!(app.notifications.unread_count(), 2);
        app.apply_gpui_action(GpuiAction::DialogsToolbar(
            DialogsToolbarAction::MarkAllNotificationsRead,
        ));
        drop(app);
        let mut app = open();
        assert_eq!(app.notifications.unread_count(), 0);
        app.apply_gpui_action(GpuiAction::DialogsToolbar(
            DialogsToolbarAction::ClearNotifications,
        ));
        drop(app);
        let mut app = open();
        assert!(app.notifications.items.is_empty());
        app.push_notification(NotificationKind::SystemNotice, "New", "", None);
        assert!(app.notifications.items[0].id > second);
    }

    #[test]
    fn restored_notification_history_keeps_the_newest_hundred_entries() {
        use crate::dialogs_toolbar_model::{
            MAX_NOTIFICATIONS, NotificationCenter, NotificationItem, NotificationKind,
        };
        let dir = tempfile::tempdir().unwrap();
        let items = (1..=102)
            .map(|id| NotificationItem {
                id,
                kind: NotificationKind::SystemNotice,
                title: id.to_string(),
                detail: String::new(),
                pane_id: None,
                timestamp_ms: 0,
                unread: true,
                count: 1,
            })
            .collect();
        let inbox = NotificationCenter { next_id: 1, items };
        std::fs::write(
            dir.path().join("notifications.json"),
            serde_json::to_vec(&inbox).unwrap(),
        )
        .unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().to_owned()), false, || {});
        assert_eq!(app.notifications.items.len(), MAX_NOTIFICATIONS);
        assert_eq!(app.notifications.items[0].id, 3);
        app.push_notification(NotificationKind::SystemNotice, "Latest", "", None);
        assert_eq!(app.notifications.items.len(), MAX_NOTIFICATIONS);
        assert_eq!(app.notifications.items.last().unwrap().id, 103);
    }

    #[test]
    fn damaged_notification_history_does_not_block_the_workspace_or_new_alerts() {
        use crate::dialogs_toolbar_model::NotificationKind;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("notifications.json");
        std::fs::write(&file, b"{incomplete").unwrap();
        let open = || Tessel::new_for_gpui_in_dir(Some(temp.path().to_owned()), false, || {});
        let mut app = open();
        assert!(app.store.is_some());
        assert!(app.notifications.items.is_empty());
        app.push_notification(NotificationKind::SystemNotice, "Recovered", "", None);
        let mut contender = open();
        assert!(contender.store.is_none());
        contender.push_notification(NotificationKind::SystemNotice, "Temporary", "", None);
        drop(contender);
        drop(app);
        assert_eq!(open().notifications.items[0].title, "Recovered");
    }

    #[test]
    fn explorer_download_dispatch_copies_the_selected_file_and_reports_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("sample.bin"), [0, 255, 9]).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(root));
        let snapshot = app.gpui_snapshot(960., 640., 4.);
        if !snapshot.toolbar.right_open || snapshot.right_tab != SideTab::Files {
            app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Files));
        }
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let key = loop {
            let snap = app.gpui_snapshot(960., 640., 4.);
            if let Some(row) = snap
                .side_panels
                .explorer
                .rows
                .iter()
                .find(|r| r.name == "sample.bin")
            {
                break row.key.clone();
            }
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let (scope, _) = app.gpui_explorer_input().unwrap();
        let action = GpuiAction::ExplorerMenu(scope, Some(key), ExplorerCommand::Download);
        app.apply_gpui_action(action.clone());
        assert!(!app.file_action_pending);
        let dest = dir.path().join("my copy.bin");
        crate::download_name::TEST_DOWNLOAD_DESTINATION.with(|next| next.set(Some(dest.clone())));
        app.apply_gpui_action(action);
        while app.file_action_pending {
            app.poll();
            assert!(std::time::Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(std::fs::read(&dest).unwrap(), [0, 255, 9]);
        assert!(
            app.gpui_snapshot(960., 640., 4.)
                .side_panels
                .explorer
                .download_notice
                .unwrap()
                .contains("my copy.bin")
        );
    }

    #[cfg(all(windows, feature = "visual-test"))]
    #[test]
    fn explorer_terminal_here_runs_in_file_parent_and_restores_its_directory() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        let nested = root.join("docs & notes");
        std::fs::create_dir_all(&nested).unwrap();
        let file = nested.join("example.txt");
        std::fs::write(&file, "selected-folder-marker").unwrap();
        let state = temp.path().join("state");
        let mut app = Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(root.clone()));
        let owner = app.state.active().unwrap().clone();
        app.state.launcher_prefs.preferred_shell = Some("cmd".into());
        let snap = app.gpui_snapshot(960., 640., 4.);
        if !snap.toolbar.right_open || snap.right_tab != SideTab::Files {
            app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Files));
        }
        app.gpui_snapshot(960., 640., 4.);
        let (scope, _) = app.gpui_explorer_input().unwrap();
        app.apply_gpui_action(GpuiAction::ExplorerMenu(
            scope,
            None,
            ExplorerCommand::TerminalHere,
        ));
        let first = app
            .active_session
            .expect("background menu creates first project terminal");
        let before = app.sessions.len();
        app.open_project_terminal_here(&owner.path, temp.path());
        assert_eq!(app.sessions.len(), before);
        app.error.clear();
        app.explorer_tree.set_filter_query("example.txt");
        let timeout = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let key = loop {
            let snap = app.gpui_snapshot(960., 640., 4.);
            if let Some(row) = snap
                .side_panels
                .explorer
                .rows
                .iter()
                .find(|r| r.name == "example.txt")
            {
                break row.key.clone();
            }
            assert!(std::time::Instant::now() < timeout);
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let (scope, _) = app.gpui_explorer_input().unwrap();
        app.apply_gpui_action(GpuiAction::ExplorerMenu(
            scope,
            Some(key),
            ExplorerCommand::TerminalHere,
        ));
        assert!(app.error.is_empty(), "{}", app.error);
        app.state.validate().unwrap();
        let launched = app.active_session.unwrap();
        assert_ne!(launched, first);
        let session = app.sessions.iter_mut().find(|s| s.id == launched).unwrap();
        assert!(session.worktree.is_none());
        assert_eq!(session.title, "CMD");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut sent = false;
        loop {
            session.poll();
            if let Some(terminal) = &mut session.terminal {
                if !sent && terminal.screen_contains(">") {
                    terminal
                        .run_quick_command("type example.txt", true)
                        .unwrap();
                    sent = true;
                }
                if terminal.screen_contains("selected-folder-marker") {
                    break;
                }
            }
            assert!(session.error.is_empty(), "{}", session.error);
            assert!(
                std::time::Instant::now() < deadline,
                "CMD did not read the relative file (command sent: {sent})"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let key = crate::terminal_layout::project_key(Some(owner.id));
        let layout = &app.state.terminal_layouts[&key];
        assert!(
            matches!(&layout.tree, Some(crate::pane_grid::PaneNode::Split {dir: crate::pane_grid::SplitDir::Col,children,..})
            if matches!(children.as_slice(),[crate::pane_grid::PaneNode::Leaf(a),crate::pane_grid::PaneNode::Leaf(b)] if a.id == first.to_string() && b.id == launched.to_string()))
        );
        drop(app);
        let restored = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        let pane = restored.sessions.iter().find(|s| s.id == launched).unwrap();
        assert_eq!(pane.project, Some(owner.id));
        assert_eq!(
            pane.cwd.canonicalize().unwrap(),
            nested.canonicalize().unwrap()
        );
    }
    #[test]
    fn git_diff_tabs_use_full_identity_and_refresh_localized_titles() {
        use crate::view_tabs_browser_model::DiffTabArea;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("example.txt");
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let deliver = |app: &mut Tessel, area, text: &str| {
            app.sender
                .send(super::super::Message::DiffDocument {
                    project: app.state.active_project,
                    source_doc: None,
                    root: dir.path().into(),
                    path: path.clone(),
                    area,
                    pinned: true,
                    result: Ok(text.into()),
                })
                .unwrap();
            app.poll();
        };
        app.apply_gpui_action(GpuiAction::SetLanguage { french: false });
        deliver(&mut app, DiffTabArea::Staged, "index");
        deliver(&mut app, DiffTabArea::Unstaged, "worktree");
        for commit in ["1234567abc", "1234567def"] {
            deliver(
                &mut app,
                DiffTabArea::Commit {
                    commit: commit.into(),
                },
                commit,
            );
        }
        assert_eq!(app.documents.len(), 4);
        assert_eq!(
            app.documents[0].diff_title.as_deref(),
            Some("example.txt (staged diff)")
        );
        assert_eq!(app.documents[2].diff_title, app.documents[3].diff_title);
        app.apply_gpui_action(GpuiAction::SetLanguage { french: true });
        assert_eq!(
            app.documents[0].diff_title.as_deref(),
            Some("example.txt (diff indexé)")
        );
        deliver(&mut app, DiffTabArea::Staged, "updated");
        deliver(
            &mut app,
            DiffTabArea::Commit {
                commit: "1234567abc".into(),
            },
            "updated commit",
        );
        assert_eq!(app.documents.len(), 4);
        assert_eq!(app.documents[0].text, "updated");
        assert_eq!(app.documents[1].text, "worktree");
        assert_eq!(app.documents[2].text, "updated commit");
        assert_eq!(app.documents[3].text, "1234567def");
        let tabs = app.gpui_snapshot(960., 640., 4.).view_tabs;
        assert!(tabs.iter().any(|t| t.title.ends_with("(diff indexé)")));
        assert!(tabs.iter().any(|t| t.title.ends_with("(diff)")));
        app.apply_gpui_action(GpuiAction::SetLanguage { french: false });
    }
    #[test]
    fn auto_save_delay_respects_last_edit_and_reports_completion_and_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("example.txt");
        std::fs::write(&path, "initial").unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let doc = crate::files::Document::open(dir.path(), &path).unwrap();
        let instance_id = doc.instance_id();
        app.documents.push(doc);
        app.active_document = 0;
        app.view = View::Files;
        app.state.editor_prefs.auto_save = true;
        app.state.editor_prefs.auto_save_delay_ms = 750;
        let edit = |app: &mut Tessel, base: &str, text: &str| {
            app.apply_gpui_action(GpuiAction::ApplyNativeDocumentEdit {
                instance_id,
                base: base.into(),
                text: text.into(),
            });
            app.documents[0].last_edited_at.unwrap()
        };
        let elapsed = std::time::Duration::from_millis;
        edit(&mut app, "initial", "one");
        let last = edit(&mut app, "one", "two");
        assert!(!app.auto_save_documents(last + elapsed(749)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "initial");
        assert!(app.auto_save_documents(last + elapsed(750)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert!(!app.documents[0].dirty());
        let last = edit(&mut app, "two", "three");
        std::fs::write(&path, "external").unwrap();
        assert!(app.auto_save_documents(last + elapsed(750)));
        assert!(app.documents[0].error.contains("changed on disk"));
        assert!(app.documents[0].dirty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert!(!app.auto_save_documents(last + elapsed(1500)));
        std::fs::write(&path, "two").unwrap();
        let last = edit(&mut app, "three", "four");
        assert!(app.auto_save_documents(last + elapsed(750)));
        assert!(app.documents[0].error.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "four");
    }
    #[cfg(windows)]
    #[test]
    fn browser_tabs_restore_each_projects_page_and_download_drawer() {
        use crate::view_tabs::Kind;
        let temp = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        let mut ids = Vec::new();
        for (name, address) in [
            ("one", "https://one.example/"),
            ("two", "https://two.example/"),
        ] {
            let dir = temp.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            app.apply_gpui_action(GpuiAction::AddProjectPath(dir));
            ids.push(app.state.active_project.unwrap());
            assert!(
                app.gpui_snapshot(960., 640., 4.)
                    .view_tabs
                    .iter()
                    .all(|t| t.kind != Kind::Browser)
            );
            app.open_tessel_url_headless(address.into());
            if name == "one" {
                app.browser.toggle_downloads_open();
            }
        }
        app.apply_gpui_action(GpuiAction::SelectProject(ids[0]));
        assert_eq!(app.browser.address_str(), "https://one.example/");
        assert!(app.browser.downloads_open());
        assert!(
            app.gpui_snapshot(960., 640., 4.)
                .view_tabs
                .iter()
                .any(|t| t.kind == Kind::Browser)
        );
        app.apply_gpui_action(GpuiAction::CloseBrowserTab);
        app.apply_gpui_action(GpuiAction::SelectProject(ids[1]));
        assert_eq!(app.browser.address_str(), "https://two.example/");
        let current_tab = app.browser.tab_id;
        app.apply_gpui_action(GpuiAction::BrowserCancelDownload {
            tab: current_tab.wrapping_sub(1),
            id: 1,
        });
        assert!(app.browser.error_message().is_empty());
        app.apply_gpui_action(GpuiAction::BrowserCancelDownload {
            tab: current_tab,
            id: 1,
        });
        assert_eq!(app.browser.error_message(), "Unknown download ID");
        assert!(!app.browser.downloads_open());
        app.apply_gpui_action(GpuiAction::SelectProject(ids[0]));
        assert!(
            app.gpui_snapshot(960., 640., 4.)
                .view_tabs
                .iter()
                .all(|t| t.kind != Kind::Browser)
        );
        app.remove_project_immediate(ids[1]);
        assert!(!app.background_browsers.contains_key(&Some(ids[1])));
    }

    #[test]
    fn explorer_bulk_file_actions_preserve_other_projects_pages_and_external_edits() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        let mut projects = Vec::new();
        for name in ["one", "two"] {
            let dir = temp.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            app.apply_gpui_action(GpuiAction::AddProjectPath(dir.clone()));
            projects.push(app.state.active_project.unwrap());
            let path = dir.join("draft.txt");
            std::fs::write(&path, "initial").unwrap();
            app.apply_gpui_action(GpuiAction::OpenDocumentPath(path));
            app.documents.last_mut().unwrap().text = name.into();
        }
        app.apply_gpui_action(GpuiAction::ExplorerSaveAll(projects[0]));
        app.apply_gpui_action(GpuiAction::ExplorerCloseAll(projects[0]));
        assert_eq!(app.documents.len(), 2);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("one/draft.txt")).unwrap(),
            "initial"
        );
        let second = temp.path().join("two/draft.txt");
        std::fs::write(&second, "external").unwrap();
        app.apply_gpui_action(GpuiAction::ExplorerSaveAll(projects[1]));
        assert!(app.documents[1].dirty());
        assert!(!app.documents[1].error.is_empty());
        assert_eq!(std::fs::read_to_string(second).unwrap(), "external");
        #[cfg(windows)]
        app.new_browser_page();
        app.apply_gpui_action(GpuiAction::ExplorerCloseAll(projects[1]));
        assert_eq!(app.pending_close, Some(1));
        app.apply_gpui_action(GpuiAction::CancelConfirmModal);
        assert_eq!(app.documents.len(), 2);
        app.apply_gpui_action(GpuiAction::ExplorerCloseAll(projects[1]));
        app.apply_gpui_action(GpuiAction::ConfirmModalSecondary);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.documents[0].text, "one");
        #[cfg(windows)]
        assert!(app.browser_tab_open);
    }

    #[test]
    fn project_tabs_isolate_names_navigation_and_bulk_close_with_dirty_buffers() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        let mut owners = Vec::new();
        for name in ["one", "two"] {
            let dir = temp.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            app.apply_gpui_action(GpuiAction::AddProjectPath(dir.clone()));
            owners.push(app.state.active_project.unwrap());
            for filename in ["notes.txt", "draft.txt"] {
                let path = dir.join(filename);
                std::fs::write(&path, "original").unwrap();
                app.apply_gpui_action(GpuiAction::OpenDocumentPath(path));
            }
        }
        app.documents[1].text = "project one draft".into();
        app.documents[3].text = "project two draft".into();
        let tabs = app.gpui_snapshot(960., 640., 4.).view_tabs;
        let files: Vec<_> = tabs
            .iter()
            .filter_map(|tab| Some((tab.doc_index?, tab.title.as_str())))
            .collect();
        assert_eq!(files, [(2, "notes.txt"), (3, "draft.txt")]);
        app.cycle_project_document(true);
        assert_eq!(app.active_document, 2);
        app.cycle_project_document(false);
        assert_eq!(app.active_document, 3);
        app.apply_gpui_action(GpuiAction::CloseDocumentTab(0));
        assert_eq!(app.documents.len(), 4);
        app.documents[2].text = "second dirty file".into();
        app.close_project_documents(Some(3));
        assert_eq!(app.pending_close, Some(2));
        app.apply_gpui_action(GpuiAction::CancelConfirmModal);
        assert!(app.remaining_close_requests.is_empty());
        assert_eq!(app.documents.len(), 4);
        app.close_project_documents(None);
        assert_eq!(app.pending_close, Some(2));
        assert_eq!(app.documents[2].text, "second dirty file");
        app.apply_gpui_action(GpuiAction::ConfirmModalSecondary);
        assert_eq!(app.pending_close, Some(2));
        assert_eq!(app.documents[2].text, "project two draft");
        app.apply_gpui_action(GpuiAction::ConfirmModalSecondary);
        assert_eq!(app.documents.len(), 2);
        assert!(app.view == View::Work);
        app.apply_gpui_action(GpuiAction::SelectProject(owners[0]));
        app.apply_gpui_action(GpuiAction::SelectDocumentTab(1));
        assert_eq!(app.documents[app.active_document].text, "project one draft");
        assert!(app.gpui_is_document_view());
    }

    #[test]
    fn checkout_tabs_scope_labels_cycling_terminal_and_bulk_close() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let feature = temp.path().join("feature");
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        for dir in [&main, &feature] {
            std::fs::create_dir(dir).unwrap();
            for name in ["notes.txt", "other.txt"] {
                std::fs::write(dir.join(name), name).unwrap();
            }
        }
        app.apply_gpui_action(GpuiAction::AddProjectPath(main.clone()));
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        let main_id = app.active_session.unwrap();
        let project = app.state.active_project;
        app.new_session_in_worktree(
            project,
            Launch::CodexChat,
            None,
            false,
            Some(crate::workspace::SavedWorktree {
                path: feature.clone(),
                branch: "feature".into(),
                base_branch: None,
            }),
        );
        let feature_id = app.active_session.unwrap();
        for (dir, name) in [
            (&main, "notes.txt"),
            (&feature, "notes.txt"),
            (&main, "other.txt"),
            (&feature, "other.txt"),
        ] {
            let mut doc = crate::files::Document::open(dir, &dir.join(name)).unwrap();
            doc.project = project;
            app.documents.push(doc);
        }
        app.documents[0].text.push_str(" unsaved");
        let files = |app: &mut Tessel| {
            app.gpui_snapshot(960., 640., 4.)
                .view_tabs
                .into_iter()
                .filter_map(|t| t.doc_index.map(|i| (i, t.title)))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            files(&mut app),
            [(1, "notes.txt".into()), (3, "other.txt".into())]
        );
        app.apply_gpui_action(GpuiAction::SelectDocumentTab(1));
        app.apply_gpui_action(GpuiAction::CloseDocumentTab(0));
        assert!(app.pending_close.is_none());
        app.cycle_project_document(true);
        assert_eq!(app.active_document, 3);
        app.close_project_documents(Some(3));
        assert!(app.pending_close.is_none());
        assert_eq!(files(&mut app), [(2, "other.txt".into())]);
        app.close_project_documents(None);
        assert!(app.view == View::Work);
        assert_eq!(app.active_session, Some(feature_id));
        assert!(files(&mut app).is_empty());
        app.apply_gpui_action(GpuiAction::FocusSession(main_id));
        assert_eq!(
            files(&mut app),
            [(0, "notes.txt".into()), (1, "other.txt".into())]
        );
        assert!(app.documents[0].dirty());
    }

    #[test]
    fn preview_replacement_stays_in_the_document_checkout() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("main");
        let feature = temp.path().join("feature");
        for root in [&project, &feature] {
            std::fs::create_dir(root).unwrap();
            for name in ["one.txt", "two.txt"] {
                std::fs::write(root.join(name), name).unwrap();
            }
        }
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(project.clone()));
        let open = |app: &mut Tessel, root: &std::path::Path, file: &str| {
            app.open_document_at_mode(root.to_owned(), root.join(file), false);
            let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !app.document_pending.is_empty() {
                app.poll_for_gpui();
                assert!(std::time::Instant::now() < until);
                std::thread::yield_now();
            }
        };
        open(&mut app, &project, "one.txt");
        let original = app.documents[app.active_document].instance_id();
        open(&mut app, &feature, "one.txt");
        open(&mut app, &feature, "two.txt");
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.documents[0].instance_id(), original);
        assert_eq!(app.documents[app.active_document].root, feature);
        assert!(app.documents[app.active_document].path.ends_with("two.txt"));
        app.sender
            .send(super::super::Message::DiffDocument {
                project: app.state.active_project,
                source_doc: None,
                root: feature.clone(),
                path: feature.join("one.txt"),
                area: crate::view_tabs_browser_model::DiffTabArea::Staged,
                pinned: false,
                result: Ok("patch".into()),
            })
            .unwrap();
        app.poll();
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.documents[0].instance_id(), original);
        assert!(app.documents[app.active_document].diff_area.is_some());
    }

    #[test]
    fn delayed_previews_and_diffs_leave_the_current_projects_editor_selected() {
        use crate::view_tabs_browser_model::DiffTabArea;
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let a = root.join("one");
        let b = root.join("two");
        for project in [&a, &b] {
            std::fs::create_dir(project).unwrap();
            std::fs::write(project.join("notes.txt"), "original").unwrap();
        }
        let mut app = Tessel::new_for_gpui_in_dir(Some(root.join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(a.clone()));
        let project_a = app.state.active_project;
        app.open_document_at_mode(a.clone(), a.join("notes.txt"), false);
        app.apply_gpui_action(GpuiAction::AddProjectPath(b.clone()));
        app.apply_gpui_action(GpuiAction::OpenDocumentPath(b.join("notes.txt")));
        app.documents[app.active_document].unpinned_preview = true;
        let current = app.documents[app.active_document].instance_id();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !app.document_pending.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            app.poll_for_gpui();
            std::thread::yield_now();
        }
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.documents[app.active_document].instance_id(), current);
        app.documents[app.active_document].text = "unsaved draft".into();
        app.sender
            .send(super::super::Message::DiffDocument {
                project: project_a,
                source_doc: None,
                root: a.clone(),
                path: a.join("notes.txt"),
                area: DiffTabArea::Staged,
                pinned: false,
                result: Ok("patch".into()),
            })
            .unwrap();
        app.poll();
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.documents[app.active_document].instance_id(), current);
        assert_eq!(app.documents[app.active_document].text, "unsaved draft");
        assert!(
            app.documents
                .iter()
                .any(|doc| doc.root == a && doc.diff_area == Some(DiffTabArea::Staged))
        );
        assert_eq!(
            std::fs::read_to_string(b.join("notes.txt")).unwrap(),
            "original"
        );
    }

    #[test]
    fn preview_link_pins_target_while_explorer_is_still_loading_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(dir.path()).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(root.join("state")), false, || {});
        let index = root.join("index.html");
        let target = root.join("report.pdf");
        std::fs::write(&index, "<a href='report.pdf'>Report</a>").unwrap();
        std::fs::write(&target, crate::pdf::fixture()).unwrap();
        let mut source = crate::files::Document::open(&root, &index).unwrap();
        source.unpinned_preview = true;
        let id = source.instance_id();
        app.documents.push(source);
        app.view = View::Files;
        app.open_document_at_mode(root.clone(), target.clone(), false);
        app.apply_gpui_action(GpuiAction::FollowDocumentLink(id, "report.pdf".into()));
        app.open_document_at_mode(root, target, false);
        assert_eq!(app.document_pending.len(), 1);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !app.document_pending.is_empty() {
            assert!(std::time::Instant::now() < deadline);
            app.poll_for_gpui();
            std::thread::yield_now();
        }
        assert_eq!(app.documents.len(), 2);
        assert!(
            app.documents[app.active_document]
                .path
                .ends_with("report.pdf")
        );
        assert!(app.documents.iter().all(|doc| !doc.unpinned_preview));
    }

    #[test]
    fn preview_link_reuses_and_pins_open_target_preserving_source() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut app = Tessel::new_for_gpui_in_dir(Some(root.join("state")), false, || {});
        for name in ["index.html", "report.txt"] {
            let path = root.join(name);
            std::fs::write(&path, name).unwrap();
            app.documents
                .push(crate::files::Document::open(root, &path).unwrap());
        }
        app.active_document = 0;
        app.documents[1].unpinned_preview = true;
        app.documents[0].unpinned_preview = true;
        app.view = View::Files;
        let source = app.documents[0].instance_id();
        let other = app.documents[1].instance_id();
        app.apply_gpui_action(GpuiAction::FollowDocumentLink(other, "report.txt".into()));
        assert_eq!(app.active_document, 0);
        assert!(app.documents[0].unpinned_preview && app.documents[1].unpinned_preview);
        app.apply_gpui_action(GpuiAction::FollowDocumentLink(source, "report.txt".into()));
        assert!(!app.documents[0].unpinned_preview);
        assert_eq!(app.documents.len(), 2);
        assert_eq!(app.active_document, 1);
        assert!(!app.documents[1].unpinned_preview);
        assert!(app.documents[0].path.ends_with("index.html"));
    }

    #[test]
    fn team_mcp_shared_poll_revalidates_real_sessions_before_persisted_mutation() {
        use crate::team_mcp_runtime::LivePane;
        use crate::team_mcp_stdio::Operation;
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };
        for case in 0..9 {
            let dir = tempfile::tempdir().unwrap();
            let project_dir = dir.path().join("project");
            std::fs::create_dir(&project_dir).unwrap();
            let data = dir.path().join("state");
            let (store, mut state) = Store::open(&data).unwrap();
            let project = state.add_project(&project_dir).unwrap();
            let cwd = state.projects[0].path.clone();
            state.sessions.push(
                serde_json::from_value(serde_json::json!({
                "id":41,"project":project,"title":"Fixture",
                "launch":"ClaudeChat","cwd":cwd}))
                .unwrap(),
            );
            state.next_pane = 42;
            store.save(&state).unwrap();
            drop(store);
            let mut app = Tessel::new_for_gpui_in_dir(Some(data.clone()), false, || {});
            assert!(!app.team_mcp.enabled());
            let pane = LivePane {
                pane: 41,
                project,
                cwd,
                local: true,
                active: true,
            };
            let launch = Arc::new(app.team_mcp.begin(&pane, &app.state).unwrap());
            // An in-memory channel models a child spawn still in progress; no CLI.
            let (_hold, receiver) = mpsc::channel();
            app.sessions
                .iter_mut()
                .find(|s| s.id == 41)
                .unwrap()
                .loading = Some(receiver);
            let child = launch.clone();
            let worker = std::thread::spawn(move || {
                child.client().call(Operation::Call {
                    name: "tessel_task_add".into(),
                    arguments: serde_json::json!({"title":"Synthetic dispatch"}),
                })
            });
            let deadline = Instant::now() + Duration::from_secs(3);
            while app.team_mcp.queued_for_test() == 0 {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            match case {
                0 => {} // live, local, matching pane: one persisted mutation
                1 => app.put_session_to_sleep(41),
                2 => app.remove_session(41),
                3 => app.sessions[0].cwd = project_dir.join("missing"),
                4 => app.sessions[0].project = Some(u64::MAX),
                5 => app.sessions[0].native_host = Some(Default::default()),
                6 => app.sessions[0].loading = None, // dormant restored pane
                7 => {
                    #[cfg(windows)]
                    let mut command = portable_pty::CommandBuilder::new("cmd.exe");
                    #[cfg(windows)]
                    command.args(["/d", "/c", "exit 0"]);
                    #[cfg(not(windows))]
                    let mut command = portable_pty::CommandBuilder::new("/bin/sh");
                    #[cfg(not(windows))]
                    command.args(["-c", "exit 0"]);
                    command.cwd(&project_dir);
                    let mut terminal = crate::terminal::Terminal::spawn_command(command).unwrap();
                    let exit_deadline = Instant::now() + Duration::from_secs(3);
                    while !terminal.exited() {
                        terminal.poll();
                        assert!(Instant::now() < exit_deadline);
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    assert!(terminal.exited());
                    app.sessions[0].loading = None;
                    app.sessions[0].terminal = Some(terminal);
                }
                _ => app.teams_hub.pending_worker_stops.push_back(41),
            }
            app.poll_for_gpui();
            let result = worker.join().unwrap();
            let loaded: Workspace =
                serde_json::from_slice(&std::fs::read(data.join("workspace.json")).unwrap())
                    .unwrap();
            if case == 0 {
                assert!(result.unwrap().is_ok());
                assert_eq!(app.state.tasks.len(), 1);
                assert_eq!(loaded.tasks, app.state.tasks);
            } else {
                assert!(result.is_err(), "case {case} retained authority");
                assert!(app.state.tasks.is_empty());
                assert!(loaded.tasks.is_empty());
            }
        }
    }

    #[test]
    fn custom_agent_rejection_keeps_draft_and_previous_settings_writable() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let persisted = || {
            let bytes = std::fs::read(dir.path().join("state/workspace.json")).unwrap();
            serde_json::from_slice::<crate::workspace::Workspace>(&bytes)
                .unwrap()
                .custom_agents
        };
        let submit = |app: &mut Tessel, name: &str| {
            app.apply_gpui_action(GpuiAction::SetCustomAgentDraft {
                name: name.into(),
                cmd: "fixture".into(),
            });
            app.apply_gpui_action(GpuiAction::SubmitNewCustomAgent);
        };
        submit(&mut app, "First Agent");
        let saved = app.state.custom_agents.clone();
        app.apply_gpui_action(GpuiAction::SetCustomAgentAccent(5));
        app.apply_gpui_action(GpuiAction::SetCustomAgentAccent(usize::MAX));
        submit(&mut app, "first AGENT");
        assert_eq!(app.state.custom_agents, saved);
        assert_eq!(persisted(), saved);
        assert_eq!(app.gpui_extra.new_custom_agent_name, "first AGENT");
        assert_eq!(app.gpui_extra.new_custom_agent_cmd, "fixture");
        assert_eq!(
            app.settings_extra.custom_error,
            "Duplicate custom agent name."
        );
        submit(&mut app, "Second Agent");
        assert!(app.settings_extra.custom_error.is_empty());
        assert_eq!(persisted().len(), 2);
        assert_eq!(persisted()[1].accent, "#5b8def");
        assert_eq!(app.gpui_extra.new_custom_agent_accent, 5);
        let id = app.state.custom_agents[0].id.clone();
        let ctx = app.gpui_extra.dummy_egui_ctx.clone();
        use crate::settings_extra_panel::SettingsExtraAction as Action;
        app.handle_settings_extra_action(&ctx, Action::ToggleCustomAgent { id, enabled: false });
        let mut edited = app.state.custom_agents[0].clone();
        edited.enabled = true;
        edited.command = "edited".into();
        app.handle_settings_extra_action(&ctx, Action::UpsertCustomAgent(edited));
        assert!(!persisted()[0].enabled);
        assert_eq!(persisted()[0].command, "edited");
        let path = dir.path().join("state/workspace.json");
        let backup = dir.path().join("previous.json");
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
        submit(&mut app, "Third Agent");
        assert_eq!(app.state.custom_agents.len(), 2);
        assert_eq!(app.gpui_extra.new_custom_agent_name, "Third Agent");
        assert!(!app.settings_extra.custom_error.is_empty());
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(backup, path).unwrap();
        assert_eq!(persisted(), app.state.custom_agents);
    }
    #[test]
    fn custom_agent_settings_refresh_launcher_and_palette_with_detected_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        for (name, command) in [
            (
                "Installed fixture",
                if cfg!(windows) { "cmd.exe /d" } else { "sh -i" },
            ),
            ("Missing fixture", "tessel-missing-command-fixture"),
        ] {
            app.apply_gpui_action(GpuiAction::SetCustomAgentDraft {
                name: name.into(),
                cmd: command.into(),
            });
            app.apply_gpui_action(GpuiAction::SubmitNewCustomAgent);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while app.agents.detecting() {
            if app.agents.poll() {
                app.detect_agents();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let snap = app.gpui_snapshot(960., 640., 4.);
        let rows: Vec<_> = snap
            .toolbar
            .installed_agents
            .iter()
            .filter(|(id, _)| id.starts_with("custom-"))
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].1, "Installed fixture");
        let names: Vec<_> = app
            .build_palette_commands(snap.theme)
            .into_iter()
            .filter(|cmd| cmd.id.starts_with("new:agent:custom-"))
            .map(|cmd| cmd.title)
            .collect();
        assert_eq!(names, ["Installed fixture"]);
    }

    #[test]
    fn generated_object_ids_keep_user_labels_out_of_ids_and_preserve_existing_objects() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("project");
        std::fs::create_dir(&project_dir).unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let project = app.state.add_project(&project_dir).unwrap();
        app.state.active_project = Some(project);
        for name in ["First / task", "Deuxième tâche"] {
            app.apply_gpui_action(GpuiAction::SetAutomationDraft {
                name: name.into(),
                prompt: "Synthetic task; never executed".into(),
                every_mins: 60,
            });
            app.apply_gpui_action(GpuiAction::SubmitNewAutomation);
        }
        assert_eq!(app.automations_store.automations.len(), 2);
        let first = &app.automations_store.automations[0];
        let second = &app.automations_store.automations[1];
        assert_eq!(first.name, "First / task");
        assert_eq!(second.name, "Deuxième tâche");
        assert_ne!(first.id, second.id);
        for automation in &app.automations_store.automations {
            let suffix = automation.id.strip_prefix("auto-").unwrap();
            assert_eq!(suffix.len(), 12);
            assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
            assert!(!automation.enabled);
            assert!(automation.runs.is_empty());
        }
        for name in ["First agent", "Second agent"] {
            app.apply_gpui_action(GpuiAction::SetCustomAgentDraft {
                name: name.into(),
                cmd: "synthetic-cli".into(),
            });
            app.apply_gpui_action(GpuiAction::SubmitNewCustomAgent);
        }
        assert_eq!(app.state.custom_agents.len(), 2);
        assert_ne!(app.state.custom_agents[0].id, app.state.custom_agents[1].id);
        assert_eq!(app.state.custom_agents[0].name, "First agent");
        assert_eq!(app.state.custom_agents[1].name, "Second agent");
        assert!(app.sessions.is_empty());
        assert!(!dir.path().join("state/automations/runs").exists());
    }

    use super::*;

    #[test]
    fn transcript_removal_requires_confirmation_and_rechecks_cached_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("profile");
        let path = root.join("projects/synthetic/12345678-1234-4234-8234-123456789abc.jsonl");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{}\n").unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        app.state.agent_prefs.insert(
            "claude".into(),
            crate::agent_preferences::Preferences {
                env: format!("CLAUDE_CONFIG_DIR={}", root.display()),
                ..Default::default()
            },
        );
        let source = app
            .session_history_sources()
            .into_iter()
            .find(|s| s.agent_id == "claude")
            .unwrap();
        let entry = crate::side_panels_model::SessionHistoryEntry {
            profile_binding: Some(source.binding),
            profile_label: String::new(),
            id: "12345678-1234-4234-8234-123456789abc".into(),
            agent_id: "claude".into(),
            title: "Synthetic".into(),
            folder: temp.path().into(),
            updated_at_ms: 0,
            file_path: Some(path.clone()),
            open_pane_id: None,
            transcript_snippet: None,
        };
        app.past_sessions_cache.push(entry.clone());
        app.apply_gpui_action(GpuiAction::SidePanels(
            crate::side_panels_ui::SidePanelsAction::DeleteSession(entry.clone()),
        ));
        assert!(app.pending_transcript_trash.is_some());
        assert!(app.gpui_has_open_modal());
        assert!(!app.transcript_trash_busy && path.exists());
        app.apply_gpui_action(GpuiAction::CancelConfirmModal);
        assert!(app.pending_transcript_trash.is_none());
        assert!(path.exists());
        app.apply_gpui_action(GpuiAction::SidePanels(
            crate::side_panels_ui::SidePanelsAction::DeleteSession(entry),
        ));
        app.past_sessions_cache.clear();
        app.apply_gpui_action(GpuiAction::ConfirmModalPrimary);
        assert!(!app.transcript_trash_busy && path.exists());
        assert!(app.error.contains("no longer available"));
    }

    #[test]
    fn native_task_board_confirmation_is_scoped_and_preserves_moved_tasks() {
        use crate::native_task_board::Command;
        use crate::tasks_board_panel::TasksBoardAction;
        let temp = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(temp.path().into()));
        app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Tasks));
        for title in ["A", "B"] {
            app.apply_gpui_action(GpuiAction::TasksBoard(TasksBoardAction::Add(title.into())));
        }
        let snap = app.gpui_snapshot(960., 640., 4.);
        let ids: Vec<_> = snap.side_panels.tasks.iter().map(|t| t.id).collect();
        for id in &ids {
            app.apply_gpui_action(GpuiAction::TasksBoard(TasksBoardAction::Move {
                id: *id,
                column: 3,
            }));
        }
        let scope = app
            .gpui_snapshot(960., 640., 4.)
            .side_panels
            .task_board
            .scope
            .unwrap();
        let action = |command| GpuiAction::NativeTaskBoard { scope, command };
        app.apply_gpui_action(action(Command::ToggleSelection));
        app.apply_gpui_action(action(Command::ToggleAll));
        app.apply_gpui_action(action(Command::RequestDelete));
        let snap = app.gpui_snapshot(960., 640., 4.);
        assert!(snap.modals.confirm_modal.unwrap().title.contains('2'));
        assert_eq!(snap.side_panels.tasks.len(), 2);
        app.apply_gpui_action(GpuiAction::CancelConfirmModal);
        assert!(app.gpui_extra.task_board.selecting);
        app.apply_gpui_action(action(Command::RequestDelete));
        // An agent moves a task while the owner's confirmation is open.
        app.apply_gpui_action(GpuiAction::TasksBoard(TasksBoardAction::Move {
            id: ids[0],
            column: 1,
        }));
        app.apply_gpui_action(GpuiAction::ConfirmModalPrimary);
        let tasks = app.gpui_snapshot(960., 640., 4.).side_panels.tasks;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, ids[0]);
        app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::History));
        app.apply_gpui_action(GpuiAction::SelectSideTab(SideTab::Tasks));
        app.apply_gpui_action(action(Command::RequestDeleteOne(ids[0])));
        assert!(
            app.gpui_snapshot(960., 640., 4.)
                .modals
                .confirm_modal
                .is_none()
        );
    }

    #[test]
    fn task_agent_show_targets_exact_pane_and_preserves_floating_terminal_on_refusal() {
        use crate::tasks_board_panel::TasksBoardAction;
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("state");
        let (store, mut state) = crate::workspace::Store::open(&data).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        let cwd = state.projects[0].path.clone();
        state.sessions.push(serde_json::from_value(serde_json::json!({"id":7,"project":project,"title":"Synthetic agent","launch":{"Agent":"codex"},"cwd":cwd})).unwrap());
        state.next_pane = 8;
        store.save(&state).unwrap();
        drop(store);
        let mut app = Tessel::new_for_gpui_in_dir(Some(data), false, || {});
        app.apply_gpui_action(GpuiAction::SelectSideTab(crate::workspace::SideTab::Tasks));
        assert!(app.apply_tasks_board_action(project, TasksBoardAction::Add("Focus".into())));
        let id = app.state.tasks[0].id;
        assert!(
            app.apply_tasks_board_action(project, TasksBoardAction::Assign { id, pane: Some(7) })
        );
        assert!(app.apply_tasks_board_action(project, TasksBoardAction::Move { id, column: 1 }));
        let scope = app
            .gpui_snapshot(960., 640., 4.)
            .side_panels
            .task_board
            .scope
            .unwrap();
        app.floating.ui.show();
        assert!(app.apply_tasks_board_action(project, TasksBoardAction::ShowAgent { id, pane: 7 }));
        assert!(app.floating.ui.is_open());
        assert!(!app.floating.ui.has_keyboard());
        assert_eq!(
            app.gpui_task_agent_focus(scope, id, 7),
            Some(GpuiFocusTarget::Terminal(Some(7)))
        );
        assert!(app.apply_tasks_board_action(project, TasksBoardAction::Move { id, column: 3 }));
        app.floating.ui.show();
        assert!(
            !app.apply_tasks_board_action(project, TasksBoardAction::ShowAgent { id, pane: 7 })
        );
        assert!(app.floating.ui.has_keyboard());
        assert_eq!(app.gpui_task_agent_focus(scope, id, 7), None);
    }

    #[test]
    fn native_task_tool_persists_current_runtime_panes_before_reply() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("state");
        let (store, mut state) = crate::workspace::Store::open(&data).unwrap();
        let project = state.add_project(dir.path()).unwrap();
        state.sessions.push(serde_json::from_value(serde_json::json!({
            "id":7,"project":project,"title":"Sleeping fixture","launch":"Shell","cwd":dir.path()
        })).unwrap());
        state.next_pane = 8;
        store.save(&state).unwrap();
        drop(store);
        let mut app = Tessel::new_for_gpui_in_dir(Some(data.clone()), false, || {});
        assert!(app.sessions[0].terminal.is_none());
        app.sessions[0].id = 41;
        app.sessions[0].title = "Current runtime pane".into();
        app.next_session = 42;
        let mut chat = crate::chat::Chat::new(project, dir.path().into());
        chat.pane = 41;
        chat.tool_calls.push_back(crate::chat::ToolCall {
            id: serde_json::json!(1),
            name: "tessel_task_add".into(),
            arguments: serde_json::json!({"title":"Synthetic task", "column":1}),
        });
        app.chats.push(chat);
        app.poll_for_gpui();
        let disk: crate::workspace::Workspace =
            serde_json::from_slice(&std::fs::read(data.join("workspace.json")).unwrap()).unwrap();
        assert_eq!(disk.next_pane, 42);
        assert_eq!(disk.sessions[0].id, 41);
        assert_eq!(disk.sessions[0].title, "Current runtime pane");
        assert_eq!(disk.tasks[0].timeline.pane_id, Some(41));
        assert_eq!(disk.tasks[0].timeline.work_periods.len(), 1);
    }

    #[test]
    #[cfg(all(windows, feature = "visual-test"))]
    fn ssh_reconnect_poll_relaunches_an_established_pane_without_a_click() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(dir.path().into()));
        let (id, _input) = app.install_terminal_find_fixture();
        let pane = app.sessions.iter_mut().find(|s| s.id == id).unwrap();
        pane.launch = Launch::ssh(crate::workspace::Host {
            alias: "-invalid-fixture".into(),
            ..Default::default()
        });
        let tmux_id = crate::claude::uuid().unwrap();
        pane.ssh_tmux_id = Some(tmux_id.clone());
        let mut channel = crate::ssh_reconnect::Channel::new(false).unwrap();
        channel.opened = Some(std::time::Instant::now());
        let mut cmd = portable_pty::CommandBuilder::new("cmd.exe");
        cmd.args(["/d", "/c", "exit 255"]);
        let mut terminal = crate::terminal::Terminal::spawn_command(cmd).unwrap();
        terminal.ssh_channel = Some(channel);
        pane.terminal = Some(terminal);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut scheduled = false;
        while pane.error.is_empty() {
            pane.poll();
            scheduled |= pane.ssh_retry.pending();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(scheduled && pane.ssh_retry.automatic());
        assert!(pane.error.contains("SSH config alias"));
        assert_eq!(pane.ssh_tmux_id, Some(tmux_id));
    }

    #[test]
    #[cfg(all(windows, feature = "visual-test"))]
    fn restart_exited_ssh_waits_for_shutdown_and_ignores_replayed_clicks() {
        use crate::terminal::Terminal;
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(dir.path().into()));
        let (id, _input) = app.install_terminal_find_fixture();
        let pane = app.sessions.iter_mut().find(|s| s.id == id).unwrap();
        pane.launch = Launch::ssh(crate::workspace::Host {
            // Reaching alias validation proves the replacement launch was attempted
            // without contacting any network host or using an SSH profile.
            alias: "-fixture".into(),
            ..Default::default()
        });
        let tmux_id = crate::claude::uuid().unwrap();
        pane.ssh_tmux_id = Some(tmux_id.clone());
        let mut command = portable_pty::CommandBuilder::new("cmd.exe");
        command.args(["/d", "/c", "exit 255"]);
        pane.terminal = Some(Terminal::spawn_command(command).unwrap());
        let action = GpuiAction::RestartSession(id);
        // A stale click does not stop a pane still considered running.
        app.apply_gpui_action(action.clone());
        assert!(
            app.sessions
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .terminal
                .is_some()
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let pane = app.sessions.iter_mut().find(|s| s.id == id).unwrap();
            pane.poll();
            if pane.terminal.as_ref().is_some_and(Terminal::exited) {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.apply_gpui_action(action.clone());
        app.apply_gpui_action(action);
        let pane = app.sessions.iter_mut().find(|s| s.id == id).unwrap();
        assert!(pane.restarting.is_some() && pane.loading.is_none());
        while pane.restarting.is_some() || pane.loading.is_some() {
            assert!(std::time::Instant::now() < deadline);
            pane.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(pane.error.contains("SSH config alias"), "{}", pane.error);
        assert_eq!(pane.ssh_tmux_id.as_ref(), Some(&tmux_id));
    }

    #[test]
    fn native_mode_switch_waits_for_confirmed_cli_exit() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        app.synthetic_native_profiles(temp.path());
        app.apply_gpui_action(GpuiAction::AddProjectPath(temp.path().into()));
        app.apply_gpui_action(GpuiAction::LaunchClaudeChat);
        app.ensure_native_chat_instances();
        let pane = app.sessions[0].id;
        let binding = app.sessions[0].profile_binding.clone();
        app.sessions[0].launch = Launch::Agent("claude".into());
        app.claude_chats.clear();
        let (sender, result) = mpsc::channel();
        app.sessions[0].restarting = Some(super::super::Restart {
            preferred_shell: None,
            switch_mode: true,
            result,
            preferences: None,
        });
        app.switch_session_chat_mode(pane);
        app.poll_for_gpui();
        assert!(matches!(app.sessions[0].launch, Launch::Agent(_)));
        assert!(app.claude_chats.is_empty());
        drop(sender);
        app.poll_for_gpui();
        assert!(matches!(app.sessions[0].launch, Launch::Agent(_)));
        assert!(app.sessions[0].error.contains("not confirmed"));
        let (sender, result) = mpsc::channel();
        app.sessions[0].restarting = Some(super::super::Restart {
            preferred_shell: None,
            switch_mode: true,
            result,
            preferences: None,
        });
        sender.send(Ok(())).unwrap();
        app.poll_for_gpui();
        app.ensure_native_chat_instances();
        assert!(matches!(app.sessions[0].launch, Launch::ClaudeChat));
        assert_eq!(app.sessions[0].profile_binding, binding);
        assert!(app.claude_chats[0].process_id().is_none());
    }

    #[test]
    fn legacy_native_profile_recovers_original_account_without_connecting() {
        use crate::settings_extra_model::{ManagedAccount, ManagedAccountsRoster};
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("state");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let (store, mut state) = Store::open(&data).unwrap();
        let project = state.add_project(temp.path()).unwrap();
        state.general_prefs.restore_workspaces = true;
        let id = "12345678-1234-4234-8234-123456789abc";
        state.sessions.push(
            serde_json::from_value(serde_json::json!({
                "id": 1, "project": project, "title": "Synthetic native",
                "launch": Launch::ClaudeChat, "cwd": state.projects[0].path
            }))
            .unwrap(),
        );
        state
            .remember_native_thread(1, project, "Claude", Some(id))
            .unwrap();
        let prefs = crate::agent_preferences::Preferences {
            command: "tessel-synthetic-cli-does-not-exist".into(),
            env: format!("CLAUDE_CONFIG_DIR={}", home.join(".claude").display()),
            ..Default::default()
        };
        state.agent_prefs.insert("claude".into(), prefs.clone());
        store.save(&state).unwrap();
        drop(store);
        let roster = ManagedAccountsRoster {
            accounts: ["original", "selected"]
                .map(|id| ManagedAccount {
                    id: id.into(),
                    label: id.into(),
                    email: String::new(),
                    plan: String::new(),
                    added_at: 0,
                })
                .to_vec(),
            selected_id: Some("selected".into()),
        };
        crate::settings_extra_model::save_managed_accounts(&data, "claude", &roster).unwrap();
        let root =
            crate::settings_extra_model::managed_account_profile_dir(&data, "claude", "original")
                .unwrap();
        let folder = root.join("projects/synthetic");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{id}.jsonl")), "{}\n").unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(data.clone()), false, || {});
        app.sessions[0].profile_migration = Some(
            crate::session_profile_migration::Migration::start(
                "claude".into(),
                id.into(),
                temp.path().into(),
                Some(data.clone()),
                home,
                prefs,
                None,
                None,
            )
            .unwrap(),
        );
        app.ensure_native_chat_instances();
        assert!(app.claude_chats[0].profile.is_none());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while app.sessions[0].profile_migration.is_some() {
            app.poll_for_gpui();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        app.ensure_native_chat_instances();
        assert!(
            app.sessions[0].error.is_empty(),
            "{}",
            app.sessions[0].error
        );
        let binding = Some(crate::session_profile::Binding::Managed {
            account_id: "original".into(),
        });
        assert_eq!(app.sessions[0].profile_binding, binding);
        assert!(app.sessions[0].terminal.is_none() && app.sessions[0].loading.is_none());
        assert!(app.claude_chats[0].process_id().is_none());
        assert!(!app.claude_chats[0].busy());
        let mut command = std::process::Command::new("synthetic");
        app.claude_chats[0]
            .profile
            .as_ref()
            .unwrap()
            .apply("claude", &mut command)
            .unwrap();
        assert_eq!(
            command
                .get_envs()
                .find(|(k, _)| *k == "CLAUDE_CONFIG_DIR")
                .unwrap()
                .1,
            Some(root.as_os_str())
        );
        app.save();
        let saved: Workspace =
            serde_json::from_slice(&std::fs::read(data.join("workspace.json")).unwrap()).unwrap();
        assert_eq!(saved.sessions[0].profile_binding, binding);
        assert_eq!(saved.native_resume(1, project, "Claude"), Some(id));
        // Native pane identities use title-case providers, unlike history entries.
        let entry = crate::side_panels_model::SessionHistoryEntry {
            id: id.into(),
            agent_id: "claude".into(),
            profile_label: String::new(),
            profile_binding: binding.clone(),
            title: "Synthetic".into(),
            folder: temp.path().into(),
            updated_at_ms: 0,
            file_path: None,
            open_pane_id: None,
            transcript_snippet: None,
        };
        assert!(
            app.transcript_source_for_trash(&entry)
                .unwrap_err()
                .contains("Close this conversation")
        );
        // Removal of the account blocks restored native panes instead of selecting another.
        crate::settings_extra_model::save_managed_accounts(
            &data,
            "claude",
            &ManagedAccountsRoster::default(),
        )
        .unwrap();
        app.claude_chats.clear();
        app.sessions[0].profile_root = None;
        app.ensure_native_chat_instances();
        assert!(app.claude_chats[0].profile.is_none());
        app.claude_chats[0].gpui_connect();
        assert!(app.claude_chats[0].process_id().is_none());
        assert!(!app.claude_chats[0].busy());
        assert!(app.sessions[0].terminal.is_none() && app.sessions[0].loading.is_none());
    }

    #[test]
    #[cfg(windows)]
    fn legacy_profile_worker_resumes_original_account_and_persists_binding() {
        use crate::settings_extra_model::{ManagedAccount, ManagedAccountsRoster};
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("state");
        let home = temp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let (store, mut state) = Store::open(&data).unwrap();
        let project = state.add_project(temp.path()).unwrap();
        state.general_prefs.restore_workspaces = true;
        let id = "12345678-1234-4234-8234-123456789abc";
        state.sessions.push(
            serde_json::from_value(serde_json::json!({
                "id": 1, "project": project, "title": "Legacy synthetic",
                "launch": Launch::Agent("claude".into()), "cwd": temp.path(),
                "conversation_id": id
            }))
            .unwrap(),
        );
        store.save(&state).unwrap();
        drop(store);
        let roster = ManagedAccountsRoster {
            accounts: ["original", "selected"]
                .map(|id| ManagedAccount {
                    id: id.into(),
                    label: id.into(),
                    email: String::new(),
                    plan: String::new(),
                    added_at: 0,
                })
                .to_vec(),
            selected_id: Some("selected".into()),
        };
        crate::settings_extra_model::save_managed_accounts(&data, "claude", &roster).unwrap();
        let root =
            crate::settings_extra_model::managed_account_profile_dir(&data, "claude", "original")
                .unwrap();
        let transcripts = root.join("projects/synthetic");
        std::fs::create_dir_all(&transcripts).unwrap();
        std::fs::write(transcripts.join(format!("{id}.jsonl")), "{}\n").unwrap();
        let script = temp.path().join("synthetic.ps1");
        let marker = temp.path().join("spawn.json");
        std::fs::write(&script, r#"
[IO.File]::WriteAllText((Join-Path $env:SYNTHETIC_ROOT 'spawn.json'), (@{args=@($args);root=$env:CLAUDE_CONFIG_DIR} | ConvertTo-Json -Compress))
"#).unwrap();
        let prefs = crate::agent_preferences::Preferences {
            command: format!("& '{}'", script.display().to_string().replace('\'', "''")),
            env: format!(
                "CLAUDE_CONFIG_DIR={}\nSYNTHETIC_ROOT={}",
                home.join(".claude").display(),
                temp.path().display()
            ),
            ..Default::default()
        };
        let mut app = Tessel::new_for_gpui_in_dir(Some(data.clone()), false, || {});
        assert!(app.sessions[0].profile_binding.is_none());
        assert!(app.sessions[0].terminal.is_none());
        app.sessions[0].profile_migration = Some(
            crate::session_profile_migration::Migration::start(
                "claude".into(),
                id.into(),
                temp.path().into(),
                Some(data.clone()),
                home.clone(),
                prefs.clone(),
                None,
                None,
            )
            .unwrap(),
        );
        assert!(!marker.exists());
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !app.sessions[0]
            .terminal
            .as_ref()
            .is_some_and(crate::terminal::Terminal::exited)
        {
            app.poll_for_gpui();
            assert!(
                app.sessions[0].error.is_empty(),
                "{}",
                app.sessions[0].error
            );
            assert!(
                std::time::Instant::now() < deadline,
                "migration/start timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let result: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
        assert_eq!(result["args"], serde_json::json!(["--resume", id]));
        assert_eq!(result["root"], root.to_string_lossy().as_ref());
        let binding = Some(crate::session_profile::Binding::Managed {
            account_id: "original".into(),
        });
        assert_eq!(app.sessions[0].profile_binding, binding);
        app.save();
        let saved: Workspace =
            serde_json::from_slice(&std::fs::read(data.join("workspace.json")).unwrap()).unwrap();
        assert_eq!(saved.sessions[0].profile_binding, binding);
        assert_eq!(saved.sessions[0].conversation_id.as_deref(), Some(id));
        // A second matching profile must leave the old pane dormant and unbound.
        app.sessions[0].terminal = None;
        app.sessions[0].profile_binding = None;
        app.sessions[0].profile_root = None;
        std::fs::remove_file(&marker).unwrap();
        let duplicate = home.join(".claude/projects/synthetic");
        std::fs::create_dir_all(&duplicate).unwrap();
        std::fs::write(duplicate.join(format!("{id}.jsonl")), "{}\n").unwrap();
        app.sessions[0].profile_migration = Some(
            crate::session_profile_migration::Migration::start(
                "claude".into(),
                id.into(),
                temp.path().into(),
                Some(data.clone()),
                home,
                prefs,
                None,
                None,
            )
            .unwrap(),
        );
        while app.sessions[0].profile_migration.is_some() {
            app.poll_for_gpui();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.sessions[0].error.contains("multiple profiles"));
        assert!(app.sessions[0].terminal.is_none() && app.sessions[0].loading.is_none());
        assert!(app.sessions[0].profile_binding.is_none());
        assert!(!marker.exists());
        app.save();
        let saved: Workspace =
            serde_json::from_slice(&std::fs::read(data.join("workspace.json")).unwrap()).unwrap();
        assert!(saved.sessions[0].profile_binding.is_none());
    }

    #[test]
    fn history_resume_sets_identity_before_start_and_deleted_account_cannot_spawn() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        let visible = temp.path().join("visible-project");
        std::fs::create_dir_all(&visible).unwrap();
        let project = app.state.add_project(&visible).unwrap();
        let cwd = temp.path().join("original-folder");
        std::fs::create_dir_all(&cwd).unwrap();
        let binding = crate::session_profile_identity::Binding::Managed {
            account_id: "removed".into(),
        };
        let id = "12345678-1234-1234-1234-123456789abc".to_owned();
        app.past_sessions_cache
            .push(crate::side_panels_model::SessionHistoryEntry {
                profile_binding: Some(binding.clone()),
                profile_label: "Removed account".into(),
                id: id.clone(),
                agent_id: "claude".into(),
                title: "Synthetic".into(),
                folder: cwd.clone(),
                updated_at_ms: 0,
                file_path: Some(temp.path().join("synthetic.jsonl")),
                open_pane_id: None,
                transcript_snippet: None,
            });
        app.apply_gpui_action(GpuiAction::SidePanels(
            crate::side_panels_ui::SidePanelsAction::ResumeSession {
                agent_id: "claude".into(),
                session_id: id.clone(),
                folder: cwd.clone(),
                profile_binding: Some(binding.clone()),
            },
        ));
        let session = app.sessions.last().unwrap();
        assert_eq!(session.conversation_id.as_deref(), Some(id.as_str()));
        assert_eq!(session.cwd, cwd);
        assert_eq!(session.project, Some(project));
        assert_eq!(app.state.active_project, Some(project));
        assert_eq!(session.profile_binding, Some(binding));
        assert!(session.resume_required);
        assert!(session.loading.is_none() && session.terminal.is_none());
        assert!(session.error.contains("no longer available"));
    }

    #[test]
    fn session_discovery_keeps_cache_until_completion_and_does_not_retry_empty_result() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.state.panels.right_open = true;
        app.state.panels.right_tab = SideTab::History;
        app.past_sessions_cache
            .push(crate::side_panels_model::SessionHistoryEntry {
                profile_label: String::new(),
                profile_binding: None,
                id: "old".into(),
                agent_id: "test".into(),
                title: "Previous".into(),
                folder: dir.path().to_path_buf(),
                updated_at_ms: 0,
                file_path: None,
                open_pane_id: None,
                transcript_snippet: None,
            });
        app.gpui_extra.session_discovery.request(None, vec![], 50);
        assert_eq!(app.past_sessions_cache.len(), 1);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while app.gpui_extra.session_discovery.busy() {
            assert!(std::time::Instant::now() < deadline);
            app.poll_for_gpui();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(app.past_sessions_cache.is_empty());
        for _ in 0..4 {
            let snapshot = app.gpui_snapshot(960., 640., 4.);
            assert!(snapshot.side_panels.past_sessions.is_empty());
            assert!(!snapshot.side_panels.sessions_loading);
            assert!(!app.gpui_extra.session_discovery.busy());
        }
    }

    #[test]
    fn rejected_task_creation_preserves_draft_and_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(dir.path().to_path_buf()));
        let pid = app.state.active_project.unwrap();
        app.state.tasks = (0..10_000)
            .map(|id| crate::workspace::Task {
                preparation: None,
                created_by: None,
                report: None,
                decision: None,
                last_waiting: Vec::new(),
                deps: Vec::new(),
                timeline: Default::default(),
                guest_by: None,
                id,
                project: pid,
                title: "existing".into(),
                column: 0,
            })
            .collect();
        app.gpui_extra.new_task_title = "keep this draft".into();
        let generation = app.gpui_extra.panel_query_generation;
        app.apply_gpui_action(GpuiAction::TasksAddFromDraft);
        assert_eq!(app.gpui_extra.new_task_title, "keep this draft");
        assert_eq!(app.gpui_extra.panel_query_generation, generation);
        assert_eq!(app.state.tasks.len(), 10_000);
        assert!(!app.error.is_empty());
    }

    #[test]
    fn worker_launch_failure_or_revocation_never_reuses_an_existing_pane() {
        for scenario in [
            "exhausted",
            "limit",
            "closed",
            "removed",
            "stopped",
            "moved",
            "entropy",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("state");
            let (store, mut state) = crate::workspace::Store::open(&data).unwrap();
            let project = state.add_project(dir.path()).unwrap();
            let cwd = state.active().unwrap().path.clone();
            state.sessions.push(
                serde_json::from_value(serde_json::json!({
                    "id":7,"project":project,"title":"Existing pane","launch":"Shell","cwd":cwd
                }))
                .unwrap(),
            );
            state.next_pane = 8;
            if scenario == "limit" {
                for id in 8..107 {
                    state.sessions.push(serde_json::from_value(serde_json::json!({
                        "id":id,"project":project,"title":"Existing pane","launch":"Shell","cwd":cwd
                    })).unwrap());
                }
                state.next_pane = 107;
            }
            store.save(&state).unwrap();
            drop(store);
            let mut app = Tessel::new_for_gpui_in_dir(Some(data), false, || {});
            app.state.orchestration_prefs.confirm_workers = false;
            let worker = app
                .teams_hub
                .request_worker_start(
                    &mut app.state,
                    7,
                    "codex",
                    Some("New worker"),
                    "Synthetic goal",
                    100,
                )
                .unwrap();
            // Never execute a real CLI, even if a context guard regresses.
            app.next_session = u64::MAX;
            app.teams_hub.auth.force_entropy_error = scenario == "entropy";
            match scenario {
                "closed" => app.sessions.clear(),
                "removed" => app.state.projects.clear(),
                "stopped" => {
                    app.teams_hub.stop_or_release_worker(&worker.id, false);
                }
                "moved" => {
                    let changed = dir.path().join("changed");
                    std::fs::create_dir(&changed).unwrap();
                    app.sessions[0].cwd = changed;
                }
                _ => {}
            }
            app.poll_for_gpui();
            let record = app
                .teams_hub
                .workers
                .iter()
                .find(|w| w.id == worker.id)
                .unwrap();
            assert!(record.worker_pane_id.is_none(), "{scenario}");
            assert_eq!(
                record.state,
                if scenario == "stopped" {
                    crate::teams_relay::WorkerState::Stopped
                } else {
                    crate::teams_relay::WorkerState::Failed
                },
                "{scenario}"
            );
            if scenario == "entropy" {
                assert!(
                    record
                        .summary
                        .as_deref()
                        .unwrap()
                        .contains("Secure random generation failed")
                );
            } else if scenario != "stopped" {
                assert_eq!(
                    record.summary.as_deref(),
                    Some(if matches!(scenario, "exhausted" | "limit") {
                        "Worker pane could not be created."
                    } else {
                        "Worker context changed before launch."
                    }),
                    "{scenario}"
                );
            }
            for session in &app.sessions {
                assert!((7..107).contains(&session.id));
                assert_eq!(session.title, "Existing pane");
                assert_ne!(session.pane_name.as_deref(), Some("New worker"));
                assert!(session.loading.is_none() && session.terminal.is_none());
            }
            assert!(app.teams_hub.pending_spawns.is_empty());
            assert!(
                app.teams_hub
                    .auth
                    .sign_request(u64::MAX, 100, "unallocated", &serde_json::json!({}))
                    .is_err()
            );
        }
    }

    #[test]
    fn worker_launch_preserves_worktree_and_accepts_resolved_folder_spellings() {
        for use_worktree in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("state");
            let (store, mut state) = crate::workspace::Store::open(&data).unwrap();
            let project = state.add_project(dir.path()).unwrap();
            let folder = if use_worktree {
                let folder = dir.path().join("worktree");
                std::fs::create_dir(&folder).unwrap();
                folder
            } else {
                dir.path().to_path_buf()
            };
            let cwd = dunce::canonicalize(&folder).unwrap();
            let worktree = use_worktree.then(|| crate::workspace::SavedWorktree {
                path: folder.canonicalize().unwrap(),
                branch: "synthetic".into(),
                base_branch: None,
            });
            state.sessions.push(
                serde_json::from_value(serde_json::json!({
                    "id":7,"project":project,"title":"Existing pane","launch":"Shell",
                    "cwd":cwd,"worktree":worktree
                }))
                .unwrap(),
            );
            state.next_pane = 8;
            store.save(&state).unwrap();
            drop(store);
            let mut app = Tessel::new_for_gpui_in_dir(Some(data), false, || {});
            app.state.orchestration_prefs.confirm_workers = false;
            app.state.orchestration_prefs.worker_mode =
                crate::settings_extra_model::WorkerMode::Chat;
            let worker = app
                .teams_hub
                .request_worker_start(
                    &mut app.state,
                    7,
                    "codex",
                    Some("New worker"),
                    "Synthetic goal",
                    100,
                )
                .unwrap();
            // Exercise the actual GPUI polling path. Native Chat creates a
            // dormant pane without starting its provider connection or a CLI.
            assert!(app.poll_for_gpui(), "the new pane must request a repaint");
            let record = app
                .teams_hub
                .workers
                .iter()
                .find(|w| w.id == worker.id)
                .unwrap();
            let id = record.worker_pane_id.expect("new worker must be bound");
            let mut chat = crate::chat::Chat::new(project, cwd.clone());
            chat.pane = id;
            chat.gpui_set_composer("Owner draft remains intact".into());
            let outgoing = chat.ready_sender_for_test();
            app.chats.push(chat);
            app.poll_for_gpui(); // bootstrap through the real shared polling path
            assert_eq!(app.sessions.len(), 2);
            let frame = outgoing.try_recv().unwrap();
            assert_eq!(frame["method"], "turn/start");
            assert_eq!(
                frame["params"]["input"][0]["text"],
                crate::teams_relay::WORKER_START_PROMPT
            );
            assert_eq!(
                app.chats[0].composer_state().1,
                "Owner draft remains intact"
            );
            app.poll_for_gpui();
            assert!(outgoing.try_recv().is_err());
            assert_eq!(
                app.teams_hub.workers[0].first_turn,
                crate::teams_relay::WorkerFirstTurn::Submitted
            );
            assert_ne!(id, 7);
            let spawned = app.sessions.iter().find(|s| s.id == id).unwrap();
            assert_eq!(spawned.worktree, worktree);
            assert!(super::super::same_worker_directory(&spawned.cwd, &cwd));
            assert!(spawned.loading.is_none() && spawned.terminal.is_none());
            let inbox = app.teams_hub.claim_team_inbox(id);
            assert_eq!(inbox.len(), 1);
            assert!(inbox[0].text.contains("Synthetic goal"));
            assert_eq!(inbox[0].from_pane, 7);
            assert!(app.teams_hub.claim_team_inbox(7).is_empty());
            assert!(app.teams_hub.claim_team_inbox(id).is_empty());
            assert_eq!(
                app.sessions.iter().find(|s| s.id == 7).unwrap().title,
                "Existing pane"
            );
            assert!(!super::super::same_worker_directory(
                &cwd,
                &dir.path().join("missing")
            ));
            assert!(!super::super::same_worker_directory(
                std::path::Path::new("."),
                &cwd
            ));
        }
    }

    #[test]
    fn worker_first_turn_respects_stop_release_context_and_transport_failures() {
        use crate::teams_relay::{WorkerFirstTurn, WorkerState};
        for scenario in [
            "stop",
            "stop_claude",
            "release",
            "sleep",
            "closed",
            "moved",
            "timeout",
            "send_error",
            "manual",
            "connecting_stop",
            "connecting_release",
            "connecting_moved",
        ] {
            let connecting = scenario.starts_with("connecting_");
            let scenario = scenario.strip_prefix("connecting_").unwrap_or(scenario);
            let dir = tempfile::tempdir().unwrap();
            let data = dir.path().join("state");
            let (store, mut state) = crate::workspace::Store::open(&data).unwrap();
            let project = state.add_project(dir.path()).unwrap();
            let cwd = state.active().unwrap().path.clone();
            state.sessions.push(
                serde_json::from_value(serde_json::json!({
                    "id":7,"project":project,"title":"Coordinator","launch":"Shell","cwd":cwd
                }))
                .unwrap(),
            );
            state.next_pane = 8;
            store.save(&state).unwrap();
            drop(store);
            let mut app = Tessel::new_for_gpui_in_dir(Some(data), false, || {});
            app.state.orchestration_prefs.confirm_workers = false;
            app.state.orchestration_prefs.worker_mode =
                crate::settings_extra_model::WorkerMode::Chat;
            let agent = if scenario == "stop_claude" {
                "claude"
            } else {
                "codex"
            };
            let worker = app
                .teams_hub
                .request_worker_start(&mut app.state, 7, agent, None, "Synthetic goal", 100)
                .unwrap();
            app.poll_for_gpui();
            let pane = app.teams_hub.workers[0].worker_pane_id.unwrap();
            if connecting {
                app.teams_hub.workers[0].first_turn = WorkerFirstTurn::Connecting;
            }
            // Every connection is an in-memory protocol sink. No real account,
            // executable discovery or provider connection is used in this test.
            let mut outgoing = Some(if scenario == "stop_claude" {
                let mut chat = crate::claude::Chat::new(project, cwd, None);
                chat.pane = pane;
                let rx = chat.ready_sender_for_test();
                app.claude_chats.push(chat);
                rx
            } else {
                let mut chat = crate::chat::Chat::new(project, cwd);
                chat.pane = pane;
                let rx = chat.ready_sender_for_test();
                app.chats.push(chat);
                rx
            });
            match scenario {
                "stop" | "stop_claude" | "release" => {
                    let tool = if scenario == "release" {
                        "team_worker_release"
                    } else {
                        "team_worker_stop"
                    };
                    app.teams_hub
                        .execute_tool(
                            &mut app.state,
                            7,
                            project,
                            tool,
                            &serde_json::json!({"worker_id":worker.id}),
                            crate::teams_relay::now_ms(),
                        )
                        .unwrap();
                }
                "sleep" => app.put_session_to_sleep(pane),
                "closed" => app.sessions.retain(|s| s.id != pane),
                "moved" => {
                    let moved = dir.path().join("moved");
                    std::fs::create_dir(&moved).unwrap();
                    app.sessions[0].cwd = moved;
                }
                "timeout" => {
                    app.teams_hub.workers[0].first_turn_started_at_ms =
                        Some(crate::teams_relay::now_ms() - 30_001)
                }
                "send_error" => {
                    outgoing.take();
                }
                "manual" => {
                    app.chats[0].gpui_set_composer("Owner submitted this turn".into());
                    app.chats[0].gpui_send_message();
                    assert_eq!(
                        outgoing.as_ref().unwrap().try_recv().unwrap()["method"],
                        "turn/start"
                    );
                }
                _ => unreachable!(),
            }
            app.poll_for_gpui();
            app.poll_for_gpui();
            let record = &app.teams_hub.workers[0];
            let cancelled = matches!(
                scenario,
                "stop" | "stop_claude" | "release" | "sleep" | "manual"
            );
            assert_eq!(
                record.first_turn,
                if cancelled {
                    WorkerFirstTurn::Cancelled
                } else {
                    WorkerFirstTurn::Failed
                },
                "{scenario}"
            );
            assert_eq!(
                record.state,
                match scenario {
                    "stop" | "stop_claude" | "sleep" => WorkerState::Stopped,
                    "release" => WorkerState::Released,
                    "manual" => WorkerState::Running,
                    _ => WorkerState::Failed,
                },
                "{scenario}"
            );
            if let Some(rx) = outgoing {
                assert!(rx.try_recv().is_err(), "{scenario}");
            }
            assert!(app.teams_hub.pending_worker_stops.is_empty());
            if scenario == "stop_claude" {
                assert!(!app.claude_chats[0].gpui_snapshot().ready);
                assert_eq!(app.claude_chats[0].status, "Stopped");
            } else if matches!(scenario, "stop" | "sleep") {
                assert!(!app.chats[0].gpui_snapshot().ready);
                assert_eq!(app.chats[0].status, "Stopped");
            } else if scenario == "release" {
                assert!(
                    app.chats[0].gpui_snapshot().ready,
                    "release must not stop an independent chat"
                );
            } else if scenario == "send_error" {
                let retry = app.chats[0].ready_sender_for_test();
                app.chats[0].error.clear();
                app.poll_for_gpui();
                assert!(
                    retry.try_recv().is_err(),
                    "a failed automatic send cannot replay after recovery"
                );
            }
        }
    }

    #[test]
    fn file_timeline_late_comparison_cannot_replace_a_new_view_or_cross_projects() {
        use super::super::{Message, TimelineOrigin, TimelineSurface};
        use crate::file_timeline::Action;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("one");
        let other = temp.path().join("two");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&other).unwrap();
        let path = root.join("notes.txt");
        std::fs::write(&path, "notes").unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(temp.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(root.clone()));
        app.apply_gpui_action(GpuiAction::OpenDocumentPath(path));
        let path = app.documents[0].path.clone();
        let project = app.state.active_project;
        app.state.editor_prefs.timeline_shown = true;
        app.timeline_snapshot(true);
        app.timeline_action(Action::TogglePin(path.clone()));
        app.select_work_tab();
        let surface = app.timeline_surface().unwrap();
        assert!(matches!(surface, TimelineSurface::Work(_)));
        let late = || Message::DiffDocument {
            project,
            source_doc: Some(TimelineOrigin::Pinned(project, surface)),
            root: root.clone(),
            path: path.clone(),
            area: crate::view_tabs_browser_model::DiffTabArea::Unstaged,
            pinned: false,
            result: Ok("old result".into()),
        };
        app.apply_gpui_action(GpuiAction::SelectDocumentTab(0));
        app.sender.send(late()).unwrap();
        app.poll();
        assert_eq!(app.documents.len(), 1);
        for view in [View::Remote, View::GuestFiles] {
            app.view = view;
            assert!(app.timeline_snapshot(true).file.is_none());
        }
        app.apply_gpui_action(GpuiAction::AddProjectPath(other));
        app.sender.send(late()).unwrap();
        app.poll();
        assert!(app.timeline_snapshot(true).file.is_none());
        assert!(app.timeline.pin.is_none());
        assert_eq!(app.documents.len(), 1);
    }

    #[test]
    fn file_timeline_filters_survive_reopen_and_default_to_both() {
        use crate::file_timeline::{Action, Source};
        let old: crate::workspace::EditorPreferences = serde_json::from_str("{}").unwrap();
        assert!(old.timeline_git && old.timeline_local);
        let dir = tempfile::tempdir().unwrap();
        let open = || Tessel::new_for_gpui_in_dir(Some(dir.path().to_owned()), false, || {});
        {
            let mut app = open();
            app.apply_gpui_action(GpuiAction::Timeline(Action::ToggleSource(Source::Git)));
            assert!(app.error.is_empty(), "{}", app.error);
        }
        let prefs = open().state.editor_prefs;
        assert!(!prefs.timeline_git && prefs.timeline_local);
    }

    #[test]
    fn outline_sort_survives_workspace_reopen_and_defaults_for_old_preferences() {
        use crate::outline_tree::Sort;
        let old: crate::workspace::EditorPreferences =
            serde_json::from_str(r#"{"compact_folders":true}"#).unwrap();
        assert_eq!(old.outline_sort, Sort::Position);
        assert!(old.outline_follow_cursor);
        let dir = tempfile::tempdir().unwrap();
        let open = || Tessel::new_for_gpui_in_dir(Some(dir.path().to_owned()), false, || {});
        {
            let mut app = open();
            app.apply_gpui_action(GpuiAction::OutlineSort(Sort::Kind));
            app.apply_gpui_action(GpuiAction::OutlineFollow(false));
            assert!(app.error.is_empty(), "{}", app.error);
        }
        let prefs = open().state.editor_prefs;
        assert_eq!(prefs.outline_sort, Sort::Kind);
        assert!(!prefs.outline_follow_cursor);
    }

    #[test]
    #[cfg(all(windows, feature = "visual-test"))]
    fn ssh_settings_removal_closes_owned_panes_including_subfolder_terminals() {
        for disconnect in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
            app.apply_gpui_action(GpuiAction::AddProjectPath(dir.path().into()));
            let first = crate::workspace::Host {
                alias: "test-host".into(),
                folder: "/srv".into(),
                ..Default::default()
            };
            let nested = crate::workspace::Host {
                folder: "/srv/tools".into(),
                ..first.clone()
            };
            app.state.hosts = vec![first.clone(), nested.clone()];
            let launches = [
                Launch::ssh(first.clone()),
                Launch::Ssh(crate::launch::SshLaunch {
                    target: first.clone(),
                    start_folder: Some(nested.folder.clone()),
                }),
                Launch::ssh(nested.clone()),
                Launch::Shell,
            ];
            let ids = launches.map(|launch| {
                let (id, _) = app.install_terminal_find_fixture();
                let pane = app.sessions.iter_mut().find(|s| s.id == id).unwrap();
                pane.terminal = None;
                pane.launch = launch;
                id
            });
            app.apply_gpui_action(GpuiAction::RemoveSshHost(1, first.clone()));
            assert!(
                ids.iter()
                    .all(|id| app.sessions.iter().any(|s| s.id == *id))
            );
            app.change_hosts(|hosts| {
                crate::host_actions::save(
                    hosts,
                    Some((0, &first)),
                    crate::workspace::Host {
                        alias: "renamed".into(),
                        folder: "/new".into(),
                        ..first.clone()
                    },
                )
            })
            .unwrap();
            app.apply_gpui_action(GpuiAction::RemoveSshHost(0, first));
            assert!(
                ids.iter()
                    .all(|id| app.sessions.iter().any(|s| s.id == *id))
            );
            let target = app.state.hosts[0].clone();
            app.apply_gpui_action(if disconnect {
                GpuiAction::SshRow(0, target.clone(), SshRowCommand::Disconnect)
            } else {
                GpuiAction::RemoveSshHost(0, target.clone())
            });
            if disconnect {
                assert_eq!(app.state.hosts, [target, nested]);
            } else {
                assert_eq!(app.state.hosts, [nested]);
            }
            for (i, id) in ids.iter().enumerate() {
                assert_eq!(app.sessions.iter().any(|s| s.id == *id), i >= 2);
            }
        }
    }

    #[test]
    fn ssh_settings_row_commands_validate_the_displayed_row_and_persist_options() {
        use SshRowCommand::*;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let mut app = Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        app.apply_gpui_action(GpuiAction::SetSshDraft {
            alias: "lab".into(),
            folder: "/first".into(),
        });
        app.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
        let original = app.state.hosts[0].clone();
        app.apply_gpui_action(GpuiAction::SshRow(0, original.clone(), TogglePersistence));
        let changed = app.state.hosts[0].clone();
        assert_eq!(changed.persistent_sessions, Some(false));
        let pane_count = app.sessions.len();
        for cmd in [
            Test,
            OpenTerminal,
            Disconnect,
            TogglePersistence,
            ToggleDaemon,
        ] {
            app.apply_gpui_action(GpuiAction::SshRow(0, original.clone(), cmd));
            assert!(matches!(app.host_notice, Some((_, true))));
            assert_eq!(app.sessions.len(), pane_count);
            assert!(app.host_tests.is_empty());
            assert_eq!(app.state.hosts[0], changed);
        }
        app.apply_gpui_action(GpuiAction::SshRow(0, changed, ToggleDaemon));
        let saved = app.state.hosts[0].clone();
        assert!(!saved.host_daemon);
        app.store = None;
        app.apply_gpui_action(GpuiAction::SshRow(0, saved.clone(), TogglePersistence));
        assert!(matches!(app.host_notice, Some((_, true))));
        assert_eq!(app.state.hosts[0], saved);
        drop(app);
        let reopened = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        assert_eq!(reopened.state.hosts[0], saved);
    }

    #[test]
    fn ssh_settings_edit_preserves_options_and_handles_cancel_and_stale_rows() {
        use crate::workspace::Host;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let mut app = Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        let original = Host {
            alias: "old".into(),
            folder: "/old".into(),
            persistent_sessions: Some(false),
            host_daemon: false,
            ..Default::default()
        };
        app.change_hosts(|hosts| crate::host_actions::save(hosts, None, original))
            .unwrap();
        let original = app.state.hosts[0].clone();
        let edit = || GpuiAction::BeginSshEdit(0, original.clone());
        let draft = || GpuiAction::SetSshDraft {
            alias: "new".into(),
            folder: "/new".into(),
        };
        app.apply_gpui_action(edit());
        assert_eq!(app.gpui_extra.new_ssh_alias, "old");
        assert_eq!(app.gpui_extra.new_ssh_folder, "/old");
        app.apply_gpui_action(draft());
        app.apply_gpui_action(GpuiAction::CancelSshEdit);
        assert!(app.gpui_extra.editing_ssh.is_none());
        assert_eq!(app.state.hosts[0], original);
        app.apply_gpui_action(edit());
        app.apply_gpui_action(draft());
        app.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
        let updated = app.state.hosts[0].clone();
        assert_eq!(updated.alias, "new");
        assert_eq!(updated.folder, "/new");
        assert_eq!(updated.target_id, original.target_id);
        assert_eq!(updated.persistent_sessions, Some(false));
        assert!(!updated.host_daemon);
        app.apply_gpui_action(GpuiAction::BeginSshEdit(0, updated.clone()));
        app.apply_gpui_action(GpuiAction::SetSshDraft {
            alias: "unsaved".into(),
            folder: String::new(),
        });
        app.change_hosts(|hosts| {
            crate::host_actions::save(
                hosts,
                Some((0, &updated)),
                Host {
                    alias: "external".into(),
                    ..updated.clone()
                },
            )
        })
        .unwrap();
        app.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
        assert!(matches!(app.host_notice, Some((_, true))));
        assert_eq!(app.gpui_extra.new_ssh_alias, "unsaved");
        drop(app);
        let reopened = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        assert_eq!(reopened.state.hosts.len(), 1);
        assert_eq!(reopened.state.hosts[0].alias, "external");
    }

    #[test]
    fn ssh_settings_confirmed_removal_checks_the_displayed_target_before_saving() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let mut app = Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        for alias in ["first", "second"] {
            app.apply_gpui_action(GpuiAction::SetSshDraft {
                alias: alias.into(),
                folder: String::new(),
            });
            app.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
        }
        let first = app.state.hosts[0].clone();
        app.apply_gpui_action(GpuiAction::RemoveSshHost(1, first.clone()));
        assert_eq!(app.state.hosts.len(), 2);
        assert!(matches!(app.host_notice, Some((_, true))));
        app.apply_gpui_action(GpuiAction::RemoveSshHost(0, first));
        let second = app.state.hosts[0].clone();
        app.store = None;
        app.apply_gpui_action(GpuiAction::RemoveSshHost(0, second.clone()));
        assert_eq!(app.state.hosts.as_slice(), std::slice::from_ref(&second));
        assert!(matches!(app.host_notice, Some((_, true))));
        drop(app);
        let reopened = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        assert_eq!(reopened.state.hosts, [second]);
    }

    #[test]
    fn ssh_settings_preserve_failed_drafts_and_the_saved_login_folder() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let mut app = Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        let set = |alias: &str, folder: &str| GpuiAction::SetSshDraft {
            alias: alias.into(),
            folder: folder.into(),
        };
        app.apply_gpui_action(set("lab", ""));
        app.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
        assert_eq!(app.state.hosts.len(), 1);
        assert!(app.state.hosts[0].folder.is_empty());
        assert!(app.gpui_extra.new_ssh_alias.is_empty());
        for (alias, folder) in [("lab", ""), ("other", "../outside")] {
            app.apply_gpui_action(set(alias, folder));
            app.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
            assert_eq!(app.state.hosts.len(), 1);
            assert_eq!(app.gpui_extra.new_ssh_folder, folder);
            assert_eq!(app.gpui_extra.new_ssh_alias, alias);
            assert!(matches!(app.host_notice, Some((_, true))));
        }
        app.store = None;
        app.apply_gpui_action(set("second", "/srv"));
        app.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
        assert_eq!(app.state.hosts.len(), 1);
        assert_eq!(app.gpui_extra.new_ssh_alias, "second");
        drop(app);
        let restored = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        assert_eq!(restored.state.hosts.len(), 1);
        assert_eq!(restored.state.hosts[0].alias, "lab");
        assert!(restored.state.hosts[0].folder.is_empty());
    }

    #[test]
    fn ssh_settings_passive_sync_runs_on_entry_and_does_not_resurrect_edits() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".ssh")).unwrap();
        std::fs::write(dir.path().join(".ssh/config"), "Host lab office\n").unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.ssh_sync_home = Some(dir.path().into());
        app.apply_gpui_action(GpuiAction::OpenSettingsPage("ssh".into()));
        app.sync_ssh_settings_visibility();
        assert!(app.host_import.is_some());
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.host_import.is_some() {
            app.poll_for_gpui();
            assert!(std::time::Instant::now() < end);
            std::thread::yield_now();
        }
        assert_eq!(app.state.hosts.len(), 2);
        let removed = app.state.hosts[0].clone();
        // A queued explicit import may not undo a removal made after it started.
        let (sender, receiver) = mpsc::channel();
        app.host_import = Some((receiver, true));
        app.remove_saved_host(0, &removed).unwrap();
        assert!(app.host_import.is_none());
        assert!(
            sender
                .send(crate::ssh_config::Discovery::default())
                .is_err()
        );
        app.sync_ssh_settings_visibility();
        assert!(app.host_import.is_none(), "no rescan on every frame");
        app.overlay = None;
        app.sync_ssh_settings_visibility();
        app.apply_gpui_action(GpuiAction::OpenSettingsPage("ssh".into()));
        app.sync_ssh_settings_visibility();
        assert!(app.host_import.is_some());
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.host_import.is_some() {
            app.poll_for_gpui();
            assert!(std::time::Instant::now() < end);
            std::thread::yield_now();
        }
        assert_eq!(app.state.hosts.len(), 1);
        assert!(
            !app.state
                .hosts
                .iter()
                .any(|host| host.alias == removed.alias)
        );
    }

    #[test]
    fn ssh_settings_show_import_progress_and_persist_the_discovered_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join(".ssh");
        std::fs::create_dir(&config).unwrap();
        std::fs::write(
            config.join("config"),
            "Host lab office\n  HostName localhost\nHost *\n  Port 22\n",
        )
        .unwrap();
        let state = dir.path().join("state");
        let mut app = Tessel::new_for_gpui_in_dir(Some(state.clone()), false, || {});
        app.overlay = Some(Overlay::Settings);
        app.apply_gpui_action(GpuiAction::SetSshDraft {
            alias: "lab".into(),
            folder: "/srv/app".into(),
        });
        app.apply_gpui_action(GpuiAction::SaveSshHostFromDraft);
        app.start_ssh_import(dir.path().into(), true);
        // A second click must not start a scan of the real user's home.
        app.apply_gpui_action(GpuiAction::ImportSshConfig);
        let settings = app.gpui_snapshot(960., 640., 4.).settings.unwrap();
        assert!(settings.can_import_ssh && settings.importing_ssh);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.host_import.is_some() {
            app.poll_for_gpui();
            assert!(std::time::Instant::now() < end);
            std::thread::yield_now();
        }
        let settings = app.gpui_snapshot(960., 640., 4.).settings.unwrap();
        assert!(!settings.importing_ssh);
        assert!(matches!(settings.ssh_notice, Some((_, false))));
        drop(app);
        let restored = Tessel::new_for_gpui_in_dir(Some(state), false, || {});
        let hosts = &restored.state.hosts;
        assert_eq!(hosts.len(), 2);
        assert!(hosts.iter().any(|h| h.alias == "office"));
        assert!(
            hosts
                .iter()
                .any(|h| h.alias == "lab" && h.folder == "/srv/app")
        );
    }

    #[test]
    fn positioned_file_links_follow_the_requested_tab_through_background_loading() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let path = project.join("two.md");
        let other = project.join("one.txt");
        std::fs::write(&other, "previous").unwrap();
        std::fs::write(&path, "a\r\n😀bc\r\nlast").unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.apply_gpui_action(GpuiAction::AddProjectPath(project.clone()));
        app.apply_gpui_action(GpuiAction::LaunchCodexChat);
        let pane = app.gpui_snapshot(960., 640., 4.).tiles[0].session_id;
        app.documents
            .push(crate::files::Document::open(&project, &other).unwrap());
        app.active_document = 0;
        let alias = project.join("nested");
        std::fs::create_dir(&alias).unwrap();
        app.apply_gpui_action(GpuiAction::ExplorerOpenContentHit {
            path: alias.join("..").join("two.md"),
            line: 3,
        });
        assert!(app.documents[0].tools.pending_scroll_line.is_none());
        let cite = |source| {
            let mut links = crate::chat::citations::parse("Codex", source)
                .unwrap()
                .links;
            GpuiAction::OpenChatCitation(pane, links.pop().unwrap().1)
        };
        app.apply_gpui_action(cite("[doc](two.md:2:4)"));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // Reproduce an older search result finishing after the newer citation.
        let mut loads = Vec::new();
        let mut other_messages = Vec::new();
        while loads.len() < 2 {
            let msg = app
                .receiver
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .unwrap();
            match msg {
                super::super::Message::Document { request, .. } => loads.push((request, msg)),
                _ => other_messages.push(msg),
            }
        }
        loads.sort_by_key(|(request, _)| std::cmp::Reverse(*request));
        let older = loads.pop().unwrap().1;
        for msg in other_messages
            .into_iter()
            .chain(loads.into_iter().map(|(_, msg)| msg))
        {
            app.sender.send(msg).unwrap();
        }
        app.poll_for_gpui();
        assert!(matches!(app.view, View::Files));
        app.apply_gpui_action(GpuiAction::SelectWorkTab);
        app.sender.send(older).unwrap();
        app.poll_for_gpui();
        assert!(app.document_pending.is_empty());
        assert!(matches!(app.view, View::Work));
        assert_eq!(app.documents.len(), 2);
        let doc = &app.documents[app.active_document];
        assert!(!doc.preview_mode && !doc.unpinned_preview);
        assert_eq!(doc.tools.pending_char_range, Some((5, 5)));
        assert_eq!(doc.tools.pending_scroll_line, Some(1));
        app.apply_gpui_action(cite(":codex-file-citation{path='two.md#line-3'}"));
        let doc = &app.documents[app.active_document];
        assert_eq!(doc.tools.pending_char_range, Some((8, 8)));
        assert_eq!(doc.tools.pending_scroll_line, Some(2));
        assert!(app.documents[0].tools.pending_scroll_line.is_none());
    }

    #[test]
    fn outline_fold_and_reveal_stay_with_the_displayed_document() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        let text = r#"{"😀": [{"nested":[1]},2]}"#;
        for name in ["one.json", "two.json"] {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            app.documents
                .push(crate::files::Document::open(dir.path(), &path).unwrap());
        }
        app.active_document = 0;
        app.view = View::Work;
        app.state.panels.right_open = true;
        app.state.panels.right_tab = SideTab::Files;
        app.apply_gpui_action(GpuiAction::OutlineToggle);
        let (id, tree) = app.build_side_panels_snapshot(false).outline.unwrap();
        let action = |row| GpuiAction::OutlineRow(id, tree.clone(), row);
        let key = tree[0].key.clone();
        app.apply_gpui_action(action(OutlineRowAction::SetAllFolds { expand: false }));
        assert!(matches!(app.view, View::Work));
        let parents = crate::outline_tree::parent_keys(&tree);
        assert_eq!(parents.len(), 3);
        assert_eq!(
            app.documents[0].tools.outline_folds,
            parents.into_iter().collect()
        );
        app.apply_gpui_action(action(OutlineRowAction::SetAllFolds { expand: true }));
        assert!(app.documents[0].tools.outline_folds.is_empty());
        app.apply_gpui_action(action(OutlineRowAction::Fold(key.clone())));
        assert!(
            app.build_side_panels_snapshot(false)
                .outline_folds
                .contains(&key)
        );
        app.apply_gpui_action(action(OutlineRowAction::Reveal(
            key.clone(),
            tree[0].range,
            true,
        )));
        assert_eq!(
            app.documents[0].tools.pending_char_range,
            Some((1, text.chars().count() - 1))
        );
        assert!(matches!(app.view, View::Files));
        app.active_document = 1;
        assert!(
            app.build_side_panels_snapshot(false)
                .outline_folds
                .is_empty()
        );
        app.apply_gpui_action(action(OutlineRowAction::Reveal(
            key.clone(),
            tree[0].range,
            true,
        )));
        assert_eq!(app.documents[1].tools.pending_char_range, None);
        app.active_document = 0;
        app.documents[0].text.insert(0, ' ');
        assert!(
            app.build_side_panels_snapshot(false)
                .outline_folds
                .contains(&key)
        );
        app.apply_gpui_action(action(OutlineRowAction::Fold(key.clone())));
        assert!(app.documents[0].tools.outline_folds.contains(&key));
    }

    #[test]
    fn sessions_modal_keeps_queries_available_without_opening_the_side_panel() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.state.panels.right_open = false;
        app.state.panels.right_tab = SideTab::Files;
        app.past_sessions_cache
            .push(crate::side_panels_model::SessionHistoryEntry {
                profile_label: String::new(),
                profile_binding: None,
                id: "synthetic".into(),
                agent_id: "test".into(),
                title: "Fixture".into(),
                folder: dir.path().to_path_buf(),
                updated_at_ms: 0,
                file_path: None,
                open_pane_id: None,
                transcript_snippet: None,
            });
        app.side_panels_ui.sessions_modal_open = true;
        assert!(app.gpui_has_open_modal());
        assert_eq!(app.build_side_panels_snapshot(false).past_sessions.len(), 1);
        let scope = app.gpui_panel_query_input().unwrap().0;
        assert_eq!(scope.target, GpuiFocusTarget::SessionsQuery);
        app.apply_gpui_action(GpuiAction::NativePanelQueryEdit {
            scope,
            base: String::new(),
            text: "Fixture".into(),
        });
        assert_eq!(app.side_panels_ui.sessions_query, "Fixture");
        assert!(!app.state.panels.right_open);
        app.side_panels_ui.notes_modal_open = true;
        assert!(app.gpui_panel_query_input().is_none());
        app.side_panels_ui.notes_modal_open = false;
        app.apply_gpui_action(GpuiAction::SidePanels(
            crate::side_panels_ui::SidePanelsAction::CloseSessionsModal,
        ));
        assert!(app.gpui_panel_query_input().is_none());
        assert!(
            app.build_side_panels_snapshot(false)
                .past_sessions
                .is_empty()
        );
    }

    #[test]
    fn sessions_modal_close_invalidates_delayed_edits_before_returning_to_history() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().join("state")), false, || {});
        app.state.panels.right_open = true;
        app.state.panels.right_tab = SideTab::History;
        app.side_panels_ui.sessions_modal_open = true;
        let scope = app.gpui_panel_query_input().unwrap().0;
        app.apply_gpui_action(GpuiAction::NativePanelQueryExitFullscreen {
            scope,
            base: String::new(),
        });
        assert!(!app.side_panels_ui.sessions_modal_open);
        assert!(app.state.panels.right_open);
        assert_ne!(app.gpui_panel_query_input().unwrap().0, scope);
        app.apply_gpui_action(GpuiAction::NativePanelQueryEdit {
            scope,
            base: String::new(),
            text: "obsolete".into(),
        });
        assert!(app.side_panels_ui.sessions_query.is_empty());
    }

    #[test]
    fn hidden_panels_do_not_read_transcripts_or_collect_rows() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = Tessel::new_for_gpui_in_dir(Some(dir.path().to_path_buf()), false, || {});
        app.past_sessions_cache
            .push(crate::side_panels_model::SessionHistoryEntry {
                profile_label: String::new(),
                profile_binding: None,
                id: "fixture".into(),
                agent_id: "test".into(),
                title: "Fixture".into(),
                folder: dir.path().to_path_buf(),
                updated_at_ms: 0,
                file_path: None,
                open_pane_id: None,
                transcript_snippet: None,
            });
        for tab in [
            SideTab::Files,
            SideTab::Changes,
            SideTab::Tasks,
            SideTab::TaskHistory,
            SideTab::Dashboard,
        ] {
            app.state.panels.right_open = true;
            app.state.panels.right_tab = tab;
            let snap = app.build_side_panels_snapshot(false);
            assert!(snap.past_sessions.is_empty());
            assert!(snap.threads.is_empty());
            assert!(
                !app.gpui_extra.conversation_loader.requested(),
                "hidden conversation scan for {tab:?}"
            );
        }
        app.state.panels.right_tab = SideTab::History;
        assert_eq!(app.build_side_panels_snapshot(false).past_sessions.len(), 1);
        app.state.panels.right_open = false;
        assert!(
            app.build_side_panels_snapshot(false)
                .past_sessions
                .is_empty()
        );
        app.state.panels.right_tab = SideTab::Conversations;
        app.build_side_panels_snapshot(false);
        assert!(!app.gpui_extra.conversation_loader.requested());
        app.state.panels.right_open = true;
        app.build_side_panels_snapshot(false);
        assert!(app.gpui_extra.conversation_loader.requested());
    }
}

#[cfg(test)]
mod document_display_tests {
    use super::Tessel;
    use std::{fs, sync::Arc};

    #[test]
    fn large_editor_snapshots_share_text_and_hunks_until_the_draft_or_baseline_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.txt");
        fs::write(&path, vec![b'x'; crate::files::MAX_LOCAL_EDIT_BYTES]).unwrap();
        let mut doc = crate::files::Document::open(dir.path(), &path).unwrap();
        let snapshot = |doc: &mut _| Tessel::build_document_summary(0, doc, Default::default());
        let first = snapshot(&mut doc);
        let idle = snapshot(&mut doc);
        assert!(Arc::ptr_eq(&first.text, &idle.text));
        assert!(!idle.dirty);
        doc.text.replace_range(1..2, "\n");
        let changed = snapshot(&mut doc);
        let again = snapshot(&mut doc);
        assert!(Arc::ptr_eq(&changed.text, &again.text));
        assert!(Arc::ptr_eq(&changed.hunk_lines, &again.hunk_lines));
        assert_eq!(changed.line_count, 2);
        assert_eq!(first.line_count, 1);
        assert_eq!(first.text.as_bytes()[1], b'x');
        assert!(!changed.hunk_lines.is_empty());
        assert!(changed.dirty);
        doc.save(None).unwrap();
        let saved = snapshot(&mut doc);
        assert!(!saved.dirty);
        assert!(saved.hunk_lines.is_empty());
        assert_eq!(saved.text.len(), crate::files::MAX_LOCAL_EDIT_BYTES);
    }
}
