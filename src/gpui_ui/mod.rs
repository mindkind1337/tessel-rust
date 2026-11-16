//! Primary GPU-accelerated renderer (`GPUI`) layered over the shared `crate::app::Tessel` core.
//!
//! Provides a 1:1 CSS Flexbox + DirectWrite subpixel typography + `resvg` vector icon frontend
//! while keeping the `eframe`/`egui` Compatibility renderer intact with automatic runtime fallback.

mod browser_input;
mod chat_inputs;
pub(crate) mod chat_view;
pub(crate) mod chrome;
mod editor_indent;
pub(crate) mod editor_input;
mod editor_syntax;
mod editor_tool_inputs;
pub mod editor_view;
mod explorer_input;
pub mod icons;
mod issues_menu;
mod linear_view;
mod modal_query_input;
pub(crate) mod modals_view;
mod panel_query_inputs;
mod question_inputs;
pub(crate) mod remote_inputs;
pub(crate) mod remote_view;
mod scm_input;
mod search_inputs;
pub(crate) mod settings_view;
pub(crate) mod sheet_view;
pub(crate) mod shell;
pub(crate) mod side_panels_view;
pub(crate) mod sidebar_view;
mod task_agent_input;
mod task_card_input;
mod task_decision_input;
mod task_rename_input;
mod task_report_view;
mod terminal_find_inputs;
pub(crate) mod terminal_view;
pub mod theme;

#[cfg(test)]
mod styled_input_tests;
#[cfg(test)]
mod tests;

use crate::app::gpui_bridge::GpuiFocusTarget as FocusInputTarget;
pub(crate) use gpui::EntityInputHandler;

use crate::{
    app::{
        Tessel,
        gpui_bridge::{GpuiAction, GpuiCenterMode},
    },
    pane_grid::{FocusDir, SplitDir},
    workspace::SideTab,
};
use gpui::{
    App, AppContext as _, Bounds, ClipboardItem, Context, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    Render, SharedString, Styled, Task, TitlebarOptions, Window, WindowBounds, WindowDecorations,
    WindowOptions, div, prelude::FluentBuilder as _, px, size,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use theme::{
    BROADCAST_BANNER_HEIGHT_PX, GpuiThemeTokens, RIGHT_PANEL_DEFAULT_WIDTH_PX,
    RIGHT_PANEL_MAX_WIDTH_PX, RIGHT_PANEL_MIN_WIDTH_PX, SIDEBAR_DEFAULT_WIDTH_PX,
    SIDEBAR_MAX_WIDTH_PX, SIDEBAR_MIN_WIDTH_PX, SIDEBAR_RAIL_WIDTH_PX, SPLIT_DIVIDER_PX,
    STATUSBAR_HEIGHT_PX, TOOLBAR_HEIGHT_PX, VIEW_TABS_HEIGHT_PX, ui_font_family,
};

#[derive(Clone, Debug)]
pub(crate) enum DragTarget {
    LeftSidebar {
        start_x: f32,
        start_width: f32,
    },
    RightPanel,
    FloatingTerminal {
        stage_height_px: f32,
    },
    SplitDivider {
        split_id: String,
        dir: SplitDir,
        index: usize,
        split_side_px: f64,
        last_pos_px: f64,
    },
}

pub(crate) struct TesselGpuiRoot {
    pub(crate) app: Tessel,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) focus_input: FocusInputTarget,
    pub(crate) left_width: f32,
    pub(crate) right_width: f32,
    pub(crate) active_drag: Option<DragTarget>,
    repaint_flag: Arc<AtomicBool>,
    presented_images: crate::gpui_pixels::PresentedImages,
    remote_inputs: remote_inputs::Inputs,
    editor_input: editor_input::Input,
    chat_inputs: chat_inputs::Inputs,
    search_inputs: search_inputs::Inputs,
    terminal_find_inputs: terminal_find_inputs::Inputs,
    browser_input: browser_input::Input,
    modal_query_input: modal_query_input::Input,
    scm_input: scm_input::Input,
    explorer_input: explorer_input::Input,
    panel_query_inputs: panel_query_inputs::Inputs,
    task_rename_input: task_rename_input::Input,
    task_card_input: task_card_input::Inputs,
    task_agent_inputs: task_agent_input::Inputs,
    task_decision_inputs: task_decision_input::Inputs,
    linear_view: linear_view::View,
    issues_menu: issues_menu::Menu,
    _poller_task: Task<()>,
}

impl TesselGpuiRoot {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let repaint_flag = Arc::new(AtomicBool::new(false));
        let cb_flag = Arc::clone(&repaint_flag);
        let app = Tessel::new_for_gpui(move || {
            cb_flag.store(true, Ordering::Release);
        });
        Self::from_app(app, repaint_flag, cx)
    }

    #[allow(dead_code)]
    pub(crate) fn new_in_dir(
        override_dir: Option<PathBuf>,
        probe_environment: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let repaint_flag = Arc::new(AtomicBool::new(false));
        let cb_flag = Arc::clone(&repaint_flag);
        let app = Tessel::new_for_gpui_in_dir(override_dir, probe_environment, move || {
            cb_flag.store(true, Ordering::Release);
        });
        Self::from_app(app, repaint_flag, cx)
    }

    fn from_app(app: Tessel, repaint_flag: Arc<AtomicBool>, cx: &mut Context<Self>) -> Self {
        private_key_input::register(cx);
        cx.bind_keys([gpui::KeyBinding::new(
            "enter",
            linear_view::SubmitKey,
            Some("TesselPrivateKey"),
        )]);
        cx.bind_keys([
            gpui::KeyBinding::new(
                "escape",
                explorer_input::Clear,
                Some("TesselExplorerFilter"),
            ),
            gpui::KeyBinding::new(
                "enter",
                explorer_input::ConsumeEnter,
                Some("TesselExplorerFilter"),
            ),
            gpui::KeyBinding::new(
                "escape",
                panel_query_inputs::Clear,
                Some("TesselPanelQuery"),
            ),
            gpui::KeyBinding::new(
                "enter",
                panel_query_inputs::ConsumeEnter,
                Some("TesselPanelQuery"),
            ),
            gpui::KeyBinding::new("enter", task_rename_input::Save, Some("TesselTaskRename")),
            gpui::KeyBinding::new(
                "escape",
                task_rename_input::Cancel,
                Some("TesselTaskRename"),
            ),
            gpui::KeyBinding::new("ctrl-enter", scm_input::Submit, Some("TesselScm")),
            gpui::KeyBinding::new("escape", scm_input::Dismiss, Some("TesselScm")),
            gpui::KeyBinding::new(
                "escape",
                modal_query_input::Dismiss,
                Some("TesselModalQuery"),
            ),
            gpui::KeyBinding::new(
                "enter",
                modal_query_input::ConsumeEnter,
                Some("TesselModalQuery"),
            ),
            gpui::KeyBinding::new(
                "escape",
                browser_input::Dismiss,
                Some("TesselBrowserAddress"),
            ),
            gpui::KeyBinding::new(
                "enter",
                browser_input::Navigate,
                Some("TesselBrowserAddress"),
            ),
            gpui::KeyBinding::new(
                "enter",
                terminal_find_inputs::Next,
                Some("TesselTerminalFind"),
            ),
            gpui::KeyBinding::new(
                "shift-enter",
                terminal_find_inputs::Previous,
                Some("TesselTerminalFind"),
            ),
            gpui::KeyBinding::new(
                "escape",
                terminal_find_inputs::Dismiss,
                Some("TesselTerminalFind"),
            ),
            gpui::KeyBinding::new(
                "enter",
                editor_tool_inputs::Submit,
                Some("TesselEditorTools"),
            ),
            gpui::KeyBinding::new(
                "shift-enter",
                editor_tool_inputs::Previous,
                Some("TesselEditorTools"),
            ),
            gpui::KeyBinding::new(
                "escape",
                editor_tool_inputs::Dismiss,
                Some("TesselEditorTools"),
            ),
            gpui::KeyBinding::new("enter", search_inputs::Accept, Some("TesselNativeSearch")),
            gpui::KeyBinding::new(
                "shift-enter",
                search_inputs::Insert,
                Some("TesselNativeSearch"),
            ),
            gpui::KeyBinding::new("up", search_inputs::Previous, Some("TesselNativeSearch")),
            gpui::KeyBinding::new("down", search_inputs::Next, Some("TesselNativeSearch")),
            gpui::KeyBinding::new("escape", search_inputs::Dismiss, Some("TesselNativeSearch")),
            gpui::KeyBinding::new(
                "enter",
                question_inputs::SubmitAnswers,
                Some("TesselChatQuestion"),
            ),
            gpui::KeyBinding::new(
                "shift-enter",
                gpui_ce_elements::editable_text::actions::Enter,
                Some("TesselChatQuestion"),
            ),
            gpui::KeyBinding::new("enter", chat_inputs::Submit, Some("TesselChatComposer")),
            gpui::KeyBinding::new(
                "shift-enter",
                gpui_ce_elements::editable_text::actions::Enter,
                Some("TesselChatComposer"),
            ),
            gpui::KeyBinding::new("tab", editor_input::Indent, Some("TesselLocalEditor")),
            gpui::KeyBinding::new(
                "shift-tab",
                editor_input::Outdent,
                Some("TesselLocalEditor"),
            ),
        ]);
        let focus_handle = cx.focus_handle();
        let poll_flag = Arc::clone(&repaint_flag);
        let poller_task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(25))
                    .await;
                let ok = this.update(cx, |root, cx| {
                    let flagged = poll_flag.swap(false, Ordering::AcqRel);
                    let worker_refresh = root.poll_backend(cx);
                    if flagged || worker_refresh {
                        cx.notify();
                    }
                });
                if ok.is_err() {
                    break;
                }
            }
        });

        Self {
            app,
            focus_handle,
            focus_input: FocusInputTarget::None,
            left_width: SIDEBAR_DEFAULT_WIDTH_PX,
            right_width: RIGHT_PANEL_DEFAULT_WIDTH_PX,
            active_drag: None,
            repaint_flag,
            presented_images: crate::gpui_pixels::PresentedImages::default(),
            remote_inputs: remote_inputs::Inputs::default(),
            editor_input: editor_input::Input::default(),
            chat_inputs: chat_inputs::Inputs::default(),
            search_inputs: search_inputs::Inputs::default(),
            terminal_find_inputs: terminal_find_inputs::Inputs::default(),
            browser_input: browser_input::Input::default(),
            modal_query_input: modal_query_input::Input::default(),
            scm_input: scm_input::Input::default(),
            explorer_input: explorer_input::Input::default(),
            panel_query_inputs: panel_query_inputs::Inputs::default(),
            task_rename_input: Default::default(),
            task_card_input: Default::default(),
            task_agent_inputs: Default::default(),
            task_decision_inputs: Default::default(),
            linear_view: Default::default(),
            issues_menu: issues_menu::Menu::new(cx),
            _poller_task: poller_task,
        }
    }

    fn poll_backend(&mut self, cx: &mut Context<Self>) -> bool {
        modal_query_input::flush(self, cx);
        task_rename_input::flush(self, cx);
        panel_query_inputs::flush(self, cx);
        explorer_input::flush(self, cx);
        scm_input::flush(self, cx);
        self.app.poll_for_gpui()
    }

    pub(crate) fn begin_drag(&mut self, drag: DragTarget, cx: &mut Context<Self>) {
        self.active_drag = Some(drag);
        cx.notify();
    }

    pub(crate) fn set_focus_input(
        &mut self,
        target: FocusInputTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_input = target;
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    pub(crate) fn dispatch_action(&mut self, action: GpuiAction, cx: &mut Context<Self>) {
        self.dispatch(action, cx);
    }

    pub(crate) fn dispatch(&mut self, action: GpuiAction, cx: &mut Context<Self>) {
        if !matches!(
            &action,
            GpuiAction::NativeModalQuery { .. }
                | GpuiAction::Share(crate::share_panel::ShareGpuiAction::GuestDraftTouch(_))
        ) {
            modal_query_input::flush(self, cx);
        }
        task_rename_input::flush(self, cx);
        panel_query_inputs::flush(self, cx);
        explorer_input::flush(self, cx);
        if !matches!(
            &action,
            GpuiAction::NativeScmEdit { .. } | GpuiAction::NativeScmTouch { .. }
        ) {
            scm_input::flush(self, cx);
        }
        if matches!(
            &action,
            GpuiAction::Remote(crate::remote_panel::control::Command::Discard)
        ) {
            self.remote_inputs.discard_text();
        }
        let remote_edit = matches!(
            &action,
            GpuiAction::Remote(
                crate::remote_panel::control::Command::Edit(_)
                    | crate::remote_panel::control::Command::Name(_)
                    | crate::remote_panel::control::Command::RenameDraft(_)
            )
        );
        let terminal_find_open = if let GpuiAction::TerminalOpenFind(sid) = &action {
            Some(*sid)
        } else {
            None
        };
        let search_toggle = matches!(
            &action,
            GpuiAction::ToggleCommandPalette | GpuiAction::OpenFinder
        );
        match &action {
            GpuiAction::NativeTaskBoard {
                command: crate::native_task_board::Command::BeginRename(_),
                ..
            } => {
                self.focus_input = FocusInputTarget::TaskRenameTitle;
            }
            GpuiAction::OpenHelp => {
                self.focus_input = FocusInputTarget::HelpQuery;
            }
            GpuiAction::OpenSettings | GpuiAction::OpenSettingsPage(_) => {
                self.focus_input = FocusInputTarget::SettingsSearch;
            }
            GpuiAction::OpenNotesModal => {
                self.focus_input = FocusInputTarget::NotesEditor;
            }
            GpuiAction::OpenGitHubDialog => {
                self.focus_input = FocusInputTarget::GitHubSearch;
            }
            GpuiAction::OpenTeamsManager(_) => {
                self.focus_input = FocusInputTarget::NewTeamName;
            }
            GpuiAction::CloseOverlays
            | GpuiAction::CloseTeamsManager
            | GpuiAction::CancelPromptModal
            | GpuiAction::CancelConfirmModal => {
                self.focus_input = FocusInputTarget::None;
            }
            _ => {}
        }
        let modal_toggle = matches!(
            &action,
            GpuiAction::OpenHelp | GpuiAction::OpenSettings | GpuiAction::OpenSettingsPage(_)
        );
        self.app.apply_gpui_action(action);
        if modal_toggle {
            self.focus_input = self
                .app
                .gpui_modal_query()
                .map(|v| v.0.1)
                .unwrap_or(FocusInputTarget::None);
        }
        if let Some(sid) = terminal_find_open
            && let Some((id, _)) = self.app.gpui_terminal_find_input(sid)
        {
            self.focus_input = FocusInputTarget::TerminalFind(sid);
            self.terminal_find_inputs.select_on_focus = Some((sid, id));
        }
        if search_toggle {
            self.focus_input =
                self.app
                    .gpui_native_search_input()
                    .map_or(FocusInputTarget::None, |(scope, _)| match scope.kind {
                        crate::app::gpui_bridge::NativeSearchKind::Palette => {
                            FocusInputTarget::PaletteQuery
                        }
                        crate::app::gpui_bridge::NativeSearchKind::Finder => {
                            FocusInputTarget::FinderQuery
                        }
                    });
        }
        if remote_edit && let Some(snapshot) = self.app.gpui_remote_input_snapshot() {
            self.remote_inputs.acknowledge(&snapshot);
        }
        if let Some(copied) = self.app.gpui_take_copied_clipboard() {
            cx.write_to_clipboard(ClipboardItem::new_string(copied));
        }
        self.repaint_flag.store(false, Ordering::Release);
        cx.notify();
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.app.linear.view.open {
            if event.keystroke.key == "escape" {
                self.app.linear.command(
                    self.app.linear.view.epoch,
                    crate::linear_model::panel::Command::Close,
                );
                if !self.app.linear.view.open {
                    self.linear_view = Default::default();
                    self.focus_handle.focus(window, cx);
                }
                cx.notify();
            } else if event.keystroke.key == "tab" {
                self.linear_view
                    .cycle(event.keystroke.modifiers.shift, window, cx);
            }
            cx.stop_propagation();
            return;
        }
        let ks = &event.keystroke;
        let ctrl = ks.modifiers.control || ks.modifiers.platform;
        let shift = ks.modifiers.shift;
        let alt = ks.modifiers.alt;
        let key = ks.key.as_str();
        if key == "escape"
            && cx
                .active_drag()
                .is_some_and(|d| d.value.is::<task_card_input::Drag>())
        {
            cx.stop_active_drag(window);
            cx.stop_propagation();
            return;
        }
        let native_target = self
            .search_inputs
            .focused(window, cx)
            .or_else(|| {
                self.explorer_input
                    .focused(window, cx)
                    .then_some(FocusInputTarget::ExplorerFilter)
            })
            .or_else(|| {
                self.task_rename_input
                    .focused(window, cx)
                    .then_some(FocusInputTarget::TaskRenameTitle)
            })
            .or_else(|| {
                self.task_decision_inputs
                    .focused(window, cx)
                    .then_some(FocusInputTarget::TaskDecisionAnswer)
            })
            .or_else(|| {
                self.task_card_input
                    .focused(window, cx)
                    .map(FocusInputTarget::TaskCard)
            })
            .or_else(|| {
                self.task_card_input
                    .confirm_focused(window)
                    .then_some(FocusInputTarget::TaskBoardConfirm)
            })
            .or_else(|| self.panel_query_inputs.focused(window, cx))
            .or_else(|| {
                self.task_card_input
                    .contains_focus(window, cx)
                    .then_some(FocusInputTarget::TaskBoardControls)
            })
            .or_else(|| self.modal_query_input.focused(window, cx))
            .or_else(|| {
                self.scm_input
                    .focused(window, cx)
                    .then_some(FocusInputTarget::ScmCommitMessage)
            })
            .or_else(|| self.editor_input.tools.focused(window, cx))
            .or_else(|| self.terminal_find_inputs.focused(window, cx))
            .or_else(|| {
                self.browser_input
                    .focused(window, cx)
                    .then_some(FocusInputTarget::BrowserUrl)
            })
            .or_else(|| self.chat_inputs.focused(window, cx))
            .or_else(|| self.remote_inputs.focused(window, cx))
            .or_else(|| {
                self.editor_input
                    .focused(window, cx)
                    .then_some(FocusInputTarget::EditorBody)
            });
        if key == "tab"
            && !ctrl
            && !alt
            && native_target == Some(FocusInputTarget::TaskBoardConfirm)
        {
            self.task_card_input.cycle_confirm(window, cx);
            cx.stop_propagation();
            return;
        }
        if key == "tab"
            && !ctrl
            && !alt
            && matches!(
                native_target,
                Some(FocusInputTarget::TaskCard(_) | FocusInputTarget::TaskBoardControls)
            )
        {
            if shift {
                window.focus_prev(cx);
            } else {
                window.focus_next(cx);
            }
            cx.stop_propagation();
            return;
        }
        if let Some(target) = native_target {
            if target == FocusInputTarget::BrowserUrl && key != "enter" && key != "escape" {
                browser_input::begin_edit(self, cx);
            }
            self.focus_input = target;
            if !ctrl && !alt && key == "enter" && target == FocusInputTarget::RemoteRename {
                self.dispatch(
                    GpuiAction::Remote(crate::remote_panel::control::Command::ConfirmRename),
                    cx,
                );
                return;
            }
        }

        if ctrl && !shift && key == "l" && self.app.gpui_browser_address().is_some() {
            if let Some(input) = self.browser_input.state().cloned()
                && input.update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
            {
                return;
            }
            browser_input::begin_edit(self, cx);
            self.focus_input = FocusInputTarget::BrowserUrl;
            self.browser_input.select_all = true;
            cx.notify();
            return;
        }
        if key == "f1" {
            self.dispatch(GpuiAction::OpenHelp, cx);
            return;
        }
        if key == "escape"
            && matches!(
                native_target,
                None | Some(FocusInputTarget::TaskCard(_) | FocusInputTarget::TaskBoardControls)
            )
            && self.app.gpui_cancel_task_selection()
        {
            cx.notify();
            return;
        }
        if key == "escape"
            && matches!(
                native_target,
                Some(
                    FocusInputTarget::PaletteQuery
                        | FocusInputTarget::FinderQuery
                        | FocusInputTarget::EditorFind
                        | FocusInputTarget::EditorReplace
                        | FocusInputTarget::EditorGoto
                        | FocusInputTarget::TerminalFind(_)
                        | FocusInputTarget::BrowserUrl
                        | FocusInputTarget::HelpQuery
                        | FocusInputTarget::SettingsSearch
                        | FocusInputTarget::ScmCommitMessage
                        | FocusInputTarget::ExplorerFilter
                        | FocusInputTarget::DashboardQuery
                        | FocusInputTarget::TaskHistoryQuery
                        | FocusInputTarget::SessionsQuery
                        | FocusInputTarget::ConversationsQuery
                        | FocusInputTarget::TaskNewTitle
                        | FocusInputTarget::TaskRenameTitle
                        | FocusInputTarget::TaskDecisionAnswer
                )
            )
        {
            return; // The scoped native Dismiss action handles IME and closing.
        }
        if key == "escape" {
            if self.app.gpui_escape_clears_modal_search() {
                cx.notify();
                return;
            }
            if self.app.gpui_has_open_modal() {
                self.dispatch(GpuiAction::CloseOverlays, cx);
                return;
            }
            if let FocusInputTarget::TerminalFind(sid) = self.focus_input {
                self.focus_input = FocusInputTarget::Terminal(sid);
                self.dispatch(GpuiAction::TerminalCloseFind(sid), cx);
                return;
            }
            if matches!(
                self.focus_input,
                FocusInputTarget::EditorFind
                    | FocusInputTarget::EditorReplace
                    | FocusInputTarget::EditorGoto
            ) {
                let was_goto = self.focus_input == FocusInputTarget::EditorGoto;
                self.focus_input = FocusInputTarget::EditorBody;
                self.dispatch(
                    if was_goto {
                        GpuiAction::ToggleEditorGoto
                    } else {
                        GpuiAction::ToggleEditorFind
                    },
                    cx,
                );
                return;
            }
        }

        if ctrl
            && shift
            && !alt
            && key == "f"
            && let FocusInputTarget::TerminalFind(sid) = self.effective_focus_target()
        {
            if let Some((id, _)) = self.app.gpui_terminal_find_input(sid)
                && let Some(state) = self.terminal_find_inputs.get(sid, id).cloned()
                && state.update(cx, |s, cx| s.marked_text_range(window, cx).is_none())
            {
                self.dispatch(GpuiAction::TerminalOpenFind(sid), cx);
            }
            return;
        }
        if ctrl && !alt {
            if key == "s" && self.focus_input == FocusInputTarget::RemoteText {
                self.dispatch(
                    GpuiAction::Remote(crate::remote_panel::control::Command::Save),
                    cx,
                );
                return;
            }
            match key {
                "k" if !shift => {
                    self.dispatch(GpuiAction::ToggleCommandPalette, cx);
                    return;
                }
                "k" if shift => {
                    self.dispatch(GpuiAction::SelectSideTab(SideTab::Tasks), cx);
                    return;
                }
                "p" if shift => {
                    self.dispatch(GpuiAction::ToggleCommandPalette, cx);
                    return;
                }
                "b" if !shift => {
                    self.dispatch(GpuiAction::ToggleLeftSidebar, cx);
                    return;
                }
                "j" if !shift => {
                    self.dispatch(GpuiAction::ToggleRightSidebar, cx);
                    return;
                }
                "j" if shift => {
                    self.dispatch(GpuiAction::OpenFinder, cx);
                    return;
                }
                "x" if shift => {
                    self.dispatch(GpuiAction::SelectSideTab(SideTab::Files), cx);
                    return;
                }
                "g" if shift => {
                    self.dispatch(GpuiAction::SelectSideTab(SideTab::Changes), cx);
                    return;
                }
                "t" if shift => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::LaunchDefaultOrShell, cx);
                    return;
                }
                "space" | " " if shift => {
                    self.dispatch(GpuiAction::ToggleLaunchMenu, cx);
                    return;
                }
                "n" if shift => {
                    self.dispatch(GpuiAction::OpenAddProjectDialog, cx);
                    return;
                }
                "`" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::ToggleFloatingTerminal, cx);
                    return;
                }
                "," if !shift => {
                    self.dispatch(GpuiAction::OpenSettings, cx);
                    return;
                }
                "=" | "+" => {
                    self.dispatch(GpuiAction::ZoomFont(1), cx);
                    return;
                }
                "-" => {
                    self.dispatch(GpuiAction::ZoomFont(-1), cx);
                    return;
                }
                "0" if !shift => {
                    self.dispatch(GpuiAction::ZoomFont(0), cx);
                    return;
                }
                "tab" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::CycleWorkspace(if shift { -1 } else { 1 }), cx);
                    return;
                }
                "s" if !shift && self.app.gpui_is_document_view() => {
                    self.dispatch(GpuiAction::SaveActiveDocument, cx);
                    return;
                }
                "f" if !shift && self.app.gpui_is_document_view() => {
                    editor_tool_inputs::open(
                        self,
                        crate::app::gpui_bridge::EditorToolField::Find,
                        window,
                        cx,
                    );
                    return;
                }
                "h" if !shift && self.app.gpui_is_document_view() => {
                    editor_tool_inputs::open(
                        self,
                        crate::app::gpui_bridge::EditorToolField::Replace,
                        window,
                        cx,
                    );
                    return;
                }
                "g" if !shift && self.app.gpui_is_document_view() => {
                    editor_tool_inputs::open(
                        self,
                        crate::app::gpui_bridge::EditorToolField::Goto,
                        window,
                        cx,
                    );
                    return;
                }
                _ => {}
            }
        }

        if alt
            && !ctrl
            && !shift
            && !matches!(
                native_target,
                Some(FocusInputTarget::TaskCard(_) | FocusInputTarget::TaskBoardControls)
            )
        {
            match key {
                "left" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::FocusPaneDirection(FocusDir::Left), cx);
                    return;
                }
                "right" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::FocusPaneDirection(FocusDir::Right), cx);
                    return;
                }
                "up" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::FocusPaneDirection(FocusDir::Up), cx);
                    return;
                }
                "down" => {
                    self.focus_input = FocusInputTarget::None;
                    self.dispatch(GpuiAction::FocusPaneDirection(FocusDir::Down), cx);
                    return;
                }
                _ => {}
            }
        }

        if let Some((scope, _)) = self.app.gpui_native_search_input() {
            if let Some(input) = self.search_inputs.get(scope.kind) {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            }
            return;
        }
        if self.app.gpui_has_open_modal() {
            let modal_target = self.focus_input;
            if !matches!(
                modal_target,
                FocusInputTarget::None | FocusInputTarget::Terminal(_)
            ) {
                let pasted = if ctrl && !alt && key == "v" {
                    cx.read_from_clipboard().and_then(|item| item.text())
                } else {
                    None
                };
                let printable = ks
                    .key_char
                    .as_deref()
                    .filter(|s| !s.is_empty() && (!ctrl || alt) && s.chars().all(|c| c >= ' '));
                if self
                    .app
                    .gpui_handle_text_target_key(modal_target, key, printable, shift, pasted)
                {
                    if let Some(copied) = self.app.gpui_take_copied_clipboard() {
                        cx.write_to_clipboard(ClipboardItem::new_string(copied));
                    }
                    cx.notify();
                }
            }
            return;
        }

        // Native fields own ordinary editing and IME input, including shortcuts.
        if native_target.is_some() {
            return;
        }
        let target = self.effective_focus_target();
        if matches!(
            target,
            FocusInputTarget::ChatComposer(_) | FocusInputTarget::ChatQuestion { .. }
        ) {
            if let Some(input) = self.chat_inputs.input_for_target(target) {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            }
            return;
        }
        if target == FocusInputTarget::EditorBody {
            if let Some(doc) = self.app.gpui_editor_document()
                && let Some(input) = self.editor_input.get(doc.instance_id())
            {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            }
            return;
        }

        if let FocusInputTarget::Terminal(sid) = target {
            if ctrl && shift && !alt && key == "f" {
                self.focus_input = FocusInputTarget::TerminalFind(sid);
                self.dispatch(GpuiAction::TerminalOpenFind(sid), cx);
                return;
            }
            if ctrl && !alt && key == "v" {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    self.dispatch(
                        GpuiAction::TerminalPaste {
                            session_id: sid,
                            text,
                        },
                        cx,
                    );
                }
                return;
            }
            if let Some(copied) =
                self.app
                    .send_gpui_terminal_key(sid, key, ks.key_char.as_deref(), ctrl, alt, shift)
            {
                cx.write_to_clipboard(ClipboardItem::new_string(copied));
            }
            cx.notify();
            return;
        }

        let pasted = if ctrl && !alt && key == "v" {
            cx.read_from_clipboard().and_then(|item| item.text())
        } else {
            None
        };
        let printable = ks
            .key_char
            .as_deref()
            .filter(|s| !s.is_empty() && (!ctrl || alt) && s.chars().all(|c| c >= ' '));
        if self
            .app
            .gpui_handle_text_target_key(target, key, printable, shift, pasted)
        {
            if let Some(copied) = self.app.gpui_take_copied_clipboard() {
                cx.write_to_clipboard(ClipboardItem::new_string(copied));
            }
            cx.notify();
        }
    }

    fn effective_focus_target(&self) -> FocusInputTarget {
        if self.focus_input == FocusInputTarget::None {
            self.app.gpui_default_focus_target()
        } else {
            self.focus_input
        }
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.active_drag.clone() else {
            return;
        };
        let viewport_w = f32::from(window.viewport_size().width);
        let viewport_h = f32::from(window.viewport_size().height);
        match drag {
            DragTarget::LeftSidebar {
                start_x,
                start_width,
            } => {
                let next = (start_width + (f32::from(event.position.x) - start_x))
                    .clamp(SIDEBAR_MIN_WIDTH_PX, SIDEBAR_MAX_WIDTH_PX);
                if (next - self.left_width).abs() > 0.5 {
                    self.left_width = next;
                    cx.notify();
                }
            }
            DragTarget::RightPanel => {
                let next = (viewport_w - f32::from(event.position.x))
                    .clamp(RIGHT_PANEL_MIN_WIDTH_PX, RIGHT_PANEL_MAX_WIDTH_PX);
                if (next - self.right_width).abs() > 0.5 {
                    self.right_width = next;
                    cx.notify();
                }
            }
            DragTarget::FloatingTerminal { stage_height_px } => {
                let bottom_bar = if self.app.state.status_bar.visible {
                    STATUSBAR_HEIGHT_PX
                } else {
                    0.0
                };
                let from_bottom = (viewport_h - bottom_bar - f32::from(event.position.y)).max(80.0);
                let ratio = (from_bottom / stage_height_px.max(120.0)).clamp(0.20, 0.85);
                self.app
                    .apply_gpui_action(GpuiAction::SetFloatingHeightRatio(ratio));
                cx.notify();
            }
            DragTarget::SplitDivider {
                split_id,
                dir,
                index,
                split_side_px,
                last_pos_px,
            } => {
                let current_pos = match dir {
                    SplitDir::Row => f64::from(event.position.x),
                    SplitDir::Col => f64::from(event.position.y),
                };
                let delta_px = current_pos - last_pos_px;
                if delta_px.abs() >= 1.0 {
                    self.active_drag = Some(DragTarget::SplitDivider {
                        split_id: split_id.clone(),
                        dir,
                        index,
                        split_side_px,
                        last_pos_px: current_pos,
                    });
                    self.app.apply_gpui_action(GpuiAction::ResizeSplitDivider {
                        split_id,
                        dir,
                        index,
                        split_side_px,
                        delta_px,
                    });
                    cx.notify();
                }
            }
        }
    }
}

impl Focusable for TesselGpuiRoot {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TesselGpuiRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(windows)]
        crate::file_actions::set_recycle_owner(window);
        self.poll_backend(cx);

        let viewport = window.viewport_size();
        let viewport_w = f32::from(viewport.width).max(320.0);
        let viewport_h = f32::from(viewport.height).max(240.0);

        let prelim = self.app.gpui_snapshot(960.0, 640.0, SPLIT_DIVIDER_PX);
        let left_px = if prelim.toolbar.left_open {
            self.left_width
        } else {
            SIDEBAR_RAIL_WIDTH_PX
        };
        let right_px = if prelim.toolbar.right_open {
            if prelim.right_fullscreen {
                (viewport_w - left_px).max(0.0)
            } else {
                self.right_width
            }
        } else {
            0.0
        };
        let mut chrome_h = TOOLBAR_HEIGHT_PX + VIEW_TABS_HEIGHT_PX;
        if prelim.toolbar.broadcast_enabled {
            chrome_h += BROADCAST_BANNER_HEIGHT_PX;
        }
        if prelim.toolbar.error_banner.is_some() {
            chrome_h += 24.0;
        }
        if prelim.status_bar.visible {
            chrome_h += STATUSBAR_HEIGHT_PX;
        }

        let grid_w = (viewport_w - left_px - right_px).max(160.0);
        let grid_h = (viewport_h - chrome_h).max(120.0);

        #[cfg(windows)]
        if prelim.center_mode == GpuiCenterMode::BrowserTab {
            self.app.browser.auto_start_if_pending(window);
        } else {
            self.app.browser.visible(false);
        }

        let snapshot = self.app.gpui_snapshot(grid_w, grid_h, SPLIT_DIVIDER_PX);
        if let FocusInputTarget::ChatQuestion { pane_id, q_idx } = self.focus_input
            && self.app.gpui_chat_question_state(pane_id, q_idx).is_none()
        {
            self.focus_input = FocusInputTarget::None;
        }
        let target = self.effective_focus_target();
        let target_pane = match target {
            FocusInputTarget::ChatComposer(pane) => Some(pane),
            FocusInputTarget::ChatQuestion { pane_id, .. } => Some(pane_id),
            _ => None,
        };
        let chat_target = target_pane
            .filter(|pane| {
                snapshot.center_mode == GpuiCenterMode::WorkGrid
                    && !self.app.gpui_has_open_modal()
                    && snapshot
                        .tiles
                        .iter()
                        .any(|t| t.session_id == *pane && t.native_chat.is_some())
            })
            .map(|_| target);
        if self.chat_inputs.focused(window, cx).is_some()
            && self.chat_inputs.focused(window, cx) != chat_target
        {
            window.focus(&self.focus_handle, cx);
        }
        if !self.app.gpui_has_open_modal() {
            self.chat_inputs.sync(
                &snapshot,
                &self.app.gpui_chat_composer_ids(),
                &self.app.gpui_chat_question_ids(),
                cx,
            );
            if let Some(target) = chat_target
                && let Some(input) = self.chat_inputs.input_for_target(target)
            {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            }
        }
        if let Some((scope, text)) = self.app.gpui_native_search_input() {
            self.search_inputs.sync(scope, text, cx);
            if let Some(input) = self.search_inputs.get(scope.kind) {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            }
        } else if self.search_inputs.focused(window, cx).is_some() {
            window.focus(&self.focus_handle, cx);
        }
        let tool_doc = snapshot
            .active_document
            .as_ref()
            .filter(|_| snapshot.center_mode == GpuiCenterMode::DocumentEditor);
        if !self.app.gpui_has_open_modal() {
            self.editor_input
                .tools
                .sync(tool_doc, &self.app.gpui_document_ids(), cx);
            if matches!(
                self.focus_input,
                FocusInputTarget::EditorFind
                    | FocusInputTarget::EditorReplace
                    | FocusInputTarget::EditorGoto
            ) && tool_doc
                .and_then(|d| {
                    self.editor_input
                        .tools
                        .for_target(d.instance_id, self.focus_input)
                })
                .is_none()
            {
                self.focus_input = FocusInputTarget::None;
            }
        }
        if self
            .editor_input
            .tools
            .focused(window, cx)
            .is_some_and(|t| {
                self.app.gpui_has_open_modal()
                    || tool_doc.is_none()
                    || t != self.effective_focus_target()
            })
        {
            window.focus(&self.focus_handle, cx);
        }
        let native_doc = snapshot.active_document.as_ref().filter(|doc| {
            snapshot.center_mode == GpuiCenterMode::DocumentEditor
                && !doc.preview_mode
                && doc.pdf_summary.is_none()
                && doc.office_summary.is_none()
                && doc.image_dims.is_none()
        });
        if (native_doc.is_none()
            || self.app.gpui_has_open_modal()
            || self.effective_focus_target() != FocusInputTarget::EditorBody)
            && self.editor_input.focused(window, cx)
        {
            window.focus(&self.focus_handle, cx);
        }
        if !self.app.gpui_has_open_modal() {
            self.editor_input
                .sync(native_doc, &self.app.gpui_document_ids(), window, cx);
            if let Some(doc) = native_doc {
                self.editor_input
                    .prepare_styles(doc, snapshot.theme, snapshot.dark);
                if self.effective_focus_target() == FocusInputTarget::EditorBody
                    && let Some(input) = self.editor_input.get(doc.instance_id)
                {
                    window.focus(&input.read(cx).focus_handle(cx), cx);
                }
                if let Some(selection) = self.app.gpui_take_editor_selection(doc.instance_id)
                    && let Some(input) = self.editor_input.get(doc.instance_id)
                {
                    input.update(cx, |state, cx| {
                        state.move_to(selection.0, cx);
                        state.select_to(selection.1, cx);
                    });
                }
            }
        }
        if !self.app.gpui_has_open_modal()
            && let Some(doc) = tool_doc
            && let Some(input) = self
                .editor_input
                .tools
                .for_target(doc.instance_id, self.effective_focus_target())
                .cloned()
        {
            window.focus(&input.read(cx).focus_handle(cx), cx);
            if let Some((id, f)) = self.editor_input.tools.select_on_focus.take()
                && id == doc.instance_id
                && self.effective_focus_target() == editor_tool_inputs::target(f)
            {
                input.update(cx, |s, cx| {
                    let len = s.as_str().encode_utf16().count();
                    s.set_selected_text_range(0..len, window, cx);
                });
            }
        }
        if (snapshot.center_mode != GpuiCenterMode::RemoteSftp || self.app.gpui_has_open_modal())
            && self.remote_inputs.focused(window, cx).is_some()
        {
            window.focus(&self.focus_handle, cx);
        }
        if snapshot.center_mode == GpuiCenterMode::RemoteSftp {
            if !self.app.gpui_has_open_modal() {
                for command in self.remote_inputs.sync(&snapshot.remote, window, cx) {
                    self.dispatch(GpuiAction::Remote(command), cx);
                }
            }
        } else {
            self.remote_inputs = remote_inputs::Inputs::default();
        }
        if !self.app.gpui_has_open_modal()
            && let FocusInputTarget::TerminalFind(sid) = self.focus_input
            && self.app.gpui_terminal_find_input(sid).is_none()
        {
            self.focus_input = FocusInputTarget::None;
        }
        let modal_was_focused = self.modal_query_input.focused(window, cx).is_some();
        if let Some((scope, text)) = self.app.gpui_modal_query().map(|(s, t)| (s, t.to_owned())) {
            if self.modal_query_input.scope() != Some(scope) {
                self.focus_input = scope.1;
            }
            self.modal_query_input.sync(scope, &text, cx);
            if self.focus_input == scope.1
                && let Some(input) = self.modal_query_input.state().cloned()
            {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            } else if modal_was_focused {
                window.focus(&self.focus_handle, cx);
            }
        } else if modal_was_focused {
            window.focus(&self.focus_handle, cx);
        }
        let cancel_card_drag = cx
            .active_drag()
            .and_then(|d| d.value.downcast_ref::<task_card_input::Drag>())
            .is_some_and(|drag| {
                snapshot.side_panels.task_board.scope != Some(drag.scope)
                    || !snapshot.side_panels.tasks.iter().any(|t| t.id == drag.id)
                    || snapshot
                        .side_panels
                        .task_board
                        .edit
                        .as_ref()
                        .is_some_and(|e| e.id == drag.id)
                    || !window.is_window_active()
                    || self.app.gpui_has_open_modal()
            });
        if cancel_card_drag {
            cx.stop_active_drag(window);
        }
        let card_was_focused = self.task_card_input.contains_focus(window, cx)
            || self.task_card_input.confirm_focused(window);
        self.task_decision_inputs.sync(
            if snapshot.side_panel.tab == crate::workspace::SideTab::Tasks
                && snapshot.toolbar.right_open
            {
                snapshot.side_panels.task_board.scope
            } else {
                None
            },
            &snapshot.side_panels.tasks,
            self.app.gpui_has_open_modal() || !window.is_window_active(),
            cx,
        );
        self.task_agent_inputs.sync(
            snapshot.side_panels.task_board.scope,
            &snapshot.side_panels.tasks,
            self.app.gpui_has_open_modal()
                || !window.is_window_active()
                || snapshot.side_panels.task_board.edit.is_some(),
            cx,
        );
        self.task_card_input.sync(
            snapshot.side_panels.task_board.scope,
            &snapshot.side_panels.tasks,
            cx,
        );
        if card_was_focused
            && !self.task_card_input.contains_focus(window, cx)
            && !self.task_card_input.confirm_focused(window)
        {
            window.focus(&self.focus_handle, cx);
        }
        self.task_card_input.sync_dialog(
            &snapshot.side_panels.task_board,
            self.app.gpui_has_open_modal(),
            window,
            cx,
        );
        let rename = self
            .app
            .gpui_task_rename_input()
            .map(|(s, id, text)| (s, id, text.to_owned()));
        let rename_was_focused = self.task_rename_input.focused(window, cx);
        let rename_ended = rename.is_none();
        self.task_rename_input.sync(rename, window, cx);
        if rename_ended && rename_was_focused {
            window.focus(&self.focus_handle, cx);
        }
        let panel_query_was_focused = self.panel_query_inputs.focused(window, cx).is_some();
        if let Some((scope, text)) = self
            .app
            .gpui_panel_query_input()
            .map(|(s, t)| (s, t.to_owned()))
        {
            self.panel_query_inputs.sync(scope, &text, cx);
            if self.focus_input == scope.target
                && let Some(input) = self.panel_query_inputs.state()
            {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            } else if panel_query_was_focused {
                window.focus(&self.focus_handle, cx);
            }
        } else if panel_query_was_focused {
            window.focus(&self.focus_handle, cx);
        }
        let explorer_was_focused = self.explorer_input.focused(window, cx);
        if let Some((scope, text)) = self
            .app
            .gpui_explorer_input()
            .map(|(s, t)| (s, t.to_owned()))
        {
            self.explorer_input.sync(scope, &text, cx);
            if self.focus_input == FocusInputTarget::ExplorerFilter
                && let Some(input) = self.explorer_input.state()
            {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            } else if explorer_was_focused {
                window.focus(&self.focus_handle, cx);
            }
        } else if explorer_was_focused {
            window.focus(&self.focus_handle, cx);
        }
        let scm_was_focused = self.scm_input.focused(window, cx);
        self.scm_input.retain(&self.app.gpui_scm_input_ids());
        if let Some((scope, text)) = self.app.gpui_scm_input().map(|(s, t)| (s, t.to_owned())) {
            self.scm_input.sync(scope, &text, cx);
            if self.focus_input == FocusInputTarget::ScmCommitMessage
                && let Some(input) = self.scm_input.state().cloned()
            {
                window.focus(&input.read(cx).focus_handle(cx), cx);
            } else if scm_was_focused {
                window.focus(&self.focus_handle, cx);
            }
        } else if scm_was_focused {
            window.focus(&self.focus_handle, cx);
        }
        let browser_was_focused = self.browser_input.focused(window, cx);
        let previous_browser_input = self.browser_input.state().cloned();
        if let Some((scope, text)) = self
            .app
            .gpui_browser_address()
            .map(|(s, t)| (s, t.to_owned()))
        {
            self.browser_input.sync(scope, &text, cx);
            if (self.browser_input.select_all
                || (browser_was_focused
                    && previous_browser_input.as_ref() != self.browser_input.state()))
                && let Some(input) = self.browser_input.state().cloned()
            {
                window.focus(&input.read(cx).focus_handle(cx), cx);
                if std::mem::take(&mut self.browser_input.select_all) {
                    input.update(cx, |s, cx| {
                        let len = s.as_str().encode_utf16().count();
                        s.set_selected_text_range(0..len, window, cx);
                    });
                }
            }
        } else if browser_was_focused {
            window.focus(&self.focus_handle, cx);
        }
        self.terminal_find_inputs.begin_frame();
        let had_terminal_find_focus = self.terminal_find_inputs.focused(window, cx).is_some();
        self.terminal_find_inputs
            .retain(&self.app.gpui_terminal_find_ids());
        if !self.app.gpui_has_open_modal() {
            for tile in &snapshot.tiles {
                if self
                    .app
                    .gpui_terminal_find_input(Some(tile.session_id))
                    .is_some()
                    && let Some(snap) = &tile.terminal_render
                {
                    self.terminal_find_inputs
                        .sync(Some(tile.session_id), snap, cx);
                }
            }
            if self.app.gpui_terminal_find_input(None).is_some()
                && let Some(snap) = &snapshot.floating.terminal_render
            {
                self.terminal_find_inputs.sync(None, snap, cx);
            }
        }
        if had_terminal_find_focus && self.terminal_find_inputs.focused(window, cx).is_none() {
            window.focus(&self.focus_handle, cx);
        }
        if let Some(FocusInputTarget::TerminalFind(sid)) =
            self.terminal_find_inputs.focused(window, cx)
            && (self.app.gpui_terminal_find_input(sid).is_none()
                || self.effective_focus_target() != FocusInputTarget::TerminalFind(sid))
        {
            window.focus(&self.focus_handle, cx);
        }
        if let FocusInputTarget::TerminalFind(sid) = self.effective_focus_target()
            && let Some((id, _)) = self.app.gpui_terminal_find_input(sid)
            && let Some(input) = self.terminal_find_inputs.get(sid, id).cloned()
        {
            window.focus(&input.read(cx).focus_handle(cx), cx);
            if self.terminal_find_inputs.select_on_focus.take() == Some((sid, id)) {
                input.update(cx, |s, cx| {
                    let len = s.as_str().encode_utf16().count();
                    s.set_selected_text_range(0..len, window, cx);
                });
            }
        }
        // Every new img() preview must participate here so discarded images leave the GPU atlas.
        let media = match snapshot.center_mode {
            GpuiCenterMode::DocumentEditor => snapshot
                .active_document
                .as_ref()
                .and_then(|doc| doc.media_pixels.clone()),
            GpuiCenterMode::RemoteSftp => snapshot.remote.pixels.clone(),
            _ => None,
        };
        for retired in self.presented_images.replace(media) {
            cx.drop_image(retired, Some(window));
        }
        if self.app.gpui_issues_menu_open() && chrome::toolbar_fit_level(&snapshot, viewport_w) != 0
        {
            self.app.apply_gpui_action(GpuiAction::ToggleIssuesMenu);
            window.focus(&self.focus_handle, cx);
        }
        self.linear_view.sync(&self.app.linear.view, window, cx);
        let tokens = GpuiThemeTokens::from_theme(snapshot.theme);
        let is_maximized = window.is_maximized();

        let focus_input = self.focus_input;

        div()
            .id("tessel-gpui-root")
            .relative()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(tokens.backdrop)
            .text_color(tokens.text)
            .font_family(ui_font_family())
            .on_action(cx.listener(linear_view::submit))
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .capture_any_mouse_up(cx.listener(|root, _: &MouseUpEvent, _, cx| {
                if root.active_drag.take().is_some() {
                    cx.notify();
                }
            }))
            .child(chrome::render_toolbar(
                &snapshot,
                &self.issues_menu,
                &tokens,
                is_maximized,
                viewport_w,
                cx,
            ))
            .when_some(
                chrome::render_broadcast_banner(&snapshot, &tokens, cx),
                |el, banner| el.child(banner),
            )
            .when_some(
                chrome::render_error_banner(&snapshot, &tokens, cx),
                |el, banner| el.child(banner),
            )
            .child(shell::render_workspace_body(
                &snapshot,
                &self.remote_inputs,
                &self.editor_input,
                &self.chat_inputs,
                &self.terminal_find_inputs,
                &self.browser_input,
                &self.scm_input,
                &self.explorer_input,
                &self.panel_query_inputs,
                &self.task_rename_input,
                &self.task_card_input,
                &self.task_agent_inputs,
                &self.task_decision_inputs,
                focus_input,
                &tokens,
                self.left_width,
                self.right_width,
                grid_h,
                window,
                cx,
            ))
            .when_some(
                chrome::render_status_bar(&snapshot, &tokens, viewport_w, cx),
                |el, bar| el.child(bar),
            )
            .children(modals_view::render_active_modals_and_popovers(
                &snapshot,
                &self.search_inputs,
                &self.modal_query_input,
                &self.task_card_input,
                &self.panel_query_inputs,
                focus_input,
                &tokens,
                cx,
            ))
            .when_some(
                settings_view::render_settings_modal(
                    &snapshot,
                    &self.modal_query_input,
                    focus_input,
                    &tokens,
                    cx,
                ),
                |el, modal| el.child(modal),
            )
            .when_some(
                issues_menu::popup(
                    &self.issues_menu,
                    self.app.gpui_issues_menu_open(),
                    snapshot.french,
                    &tokens,
                    cx,
                ),
                |el, menu| el.child(menu),
            )
            .when_some(
                linear_view::render(
                    &self.app.linear.view,
                    &mut self.linear_view,
                    snapshot.french,
                    &tokens,
                    cx,
                ),
                |el, modal| el.child(modal),
            )
    }
}

/// Launches the primary `GPUI` native hardware-accelerated window.
///
/// Returns `Err(reason)` cleanly if platform/GPU initialization or window creation fails so
/// `main.rs` can automatically fall back to the `eframe`/`egui` Compatibility renderer.
pub fn run_gpui_app() -> Result<(), String> {
    let open_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let open_error_slot = Arc::clone(&open_error);

    let run_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        gpui_platform::application().run(move |cx: &mut App| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
            let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some(SharedString::new_static("Tessel Rust")),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(640.0), px(420.0))),
                window_decorations: Some(WindowDecorations::Client),
                app_id: Some("com.tessel.rust".to_owned()),
                ..Default::default()
            };

            match cx.open_window(options, |window, cx| {
                let view = cx.new(TesselGpuiRoot::new);
                let focus = view.read(cx).focus_handle.clone();
                window.focus(&focus, cx);
                view
            }) {
                Ok(_) => {
                    cx.activate(true);
                }
                Err(err) => {
                    if let Ok(mut guard) = open_error_slot.lock() {
                        *guard = Some(format!("GPUI window initialization failed: {err}"));
                    }
                    cx.quit();
                }
            }
        });
    }));

    if let Err(payload) = run_result {
        let msg = if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_owned()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown GPUI platform panic".to_owned()
        };
        return Err(format!("GPUI platform startup panicked: {msg}"));
    }

    if let Ok(mut guard) = open_error.lock()
        && let Some(err) = guard.take()
    {
        return Err(err);
    }

    Ok(())
}

#[allow(dead_code)]
mod private_key_storage;

#[allow(dead_code)]
mod private_key_input;
