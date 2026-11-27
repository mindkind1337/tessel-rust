use super::{Launch, Terminal, Tessel, View, launch, workspace};
use crate::floating_terminal_panel::{
    FloatingTerminalAction as Action, FloatingTerminalInput, FloatingTerminalPanel as Panel,
    FloatingTerminalState,
};
use crate::floating_terminal_runtime::{Controller, Phase, Process, Refused};
use eframe::egui;

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn until(state: &mut State, phase: Phase) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.runtime.phase() != phase {
            assert!(
                Instant::now() < deadline,
                "floating phase {:?}: {:?}",
                state.runtime.phase(),
                state.runtime.error()
            );
            state.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn disconnected_drawer() -> State {
        let mut state = State::new_with_repaint(
            || {},
            workspace::FloatingPreferences {
                open: true,
                height: 0.45,
            },
        );
        let host = workspace::Host {
            alias: "-fixture-only".into(),
            folder: "/original".into(),
            ..Default::default()
        };
        state.pending_launch = Some(FloatingLaunch {
            launch: Launch::Ssh(host),
            cwd: std::env::temp_dir(),
            preferred_shell: None,
            tmux_id: Some(crate::claude::uuid().unwrap()),
            scrollback: 100,
        });
        state
            .runtime
            .open(|| {
                let mut cmd = portable_pty::CommandBuilder::new("cmd.exe");
                cmd.args(["/d", "/c", "exit 255"]);
                let mut terminal = Terminal::spawn_command(cmd).map_err(|e| e.to_string())?;
                let mut channel = crate::ssh_reconnect::Channel::new(false)?;
                channel.opened = Some(Instant::now());
                terminal.ssh_channel = Some(channel);
                Ok(terminal)
            })
            .unwrap();
        until(&mut state, Phase::Ready);
        state.ui.hide();
        let end = Instant::now() + Duration::from_secs(10);
        while !state.ssh_retry.pending() {
            state.poll();
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(5));
        }
        state
    }

    #[test]
    fn hidden_ssh_drawer_retries_its_original_target() {
        let mut state = disconnected_drawer();
        assert_eq!(state.title_str(), "SSH · -fixture-only");
        assert_eq!(
            state.status_notice(true),
            Some("Nouvelle tentative automatique prévue.")
        );
        until(&mut state, Phase::Closed);
        // Validation of the saved alias proves the SSH path ran without connecting to a server.
        assert!(state.runtime.error().unwrap().contains("SSH config alias"));
        assert!(!state.ui.is_open());
        assert!(state.active_launch.is_none());
    }

    #[test]
    fn closing_ssh_drawer_during_backoff_discards_scheduled_relaunch() {
        let mut state = disconnected_drawer();
        state.runtime.close().unwrap();
        until(&mut state, Phase::Closed);
        state.poll();
        assert!(!state.ssh_retry.pending());
        assert!(state.active_launch.is_none());
        assert!(state.runtime.error().is_none());
    }

    #[test]
    fn floating_real_shell_survives_hide_and_closes_through_verified_adapter() {
        let ctx = egui::Context::default();
        let mut state = State::new(
            &ctx,
            workspace::FloatingPreferences {
                open: true,
                height: 0.45,
            },
        );
        // Loading saved UI metadata must not create a process.
        assert_eq!(state.runtime.phase(), Phase::Closed);
        state
            .runtime
            .open(|| Terminal::spawn().map_err(|error| error.to_string()))
            .unwrap();
        until(&mut state, Phase::Ready);
        state.ui.hide();
        state.poll();
        assert_eq!(state.runtime.phase(), Phase::Ready);
        assert!(!state.runtime.process().unwrap().exited());
        state.ui.show();
        state.runtime.close().unwrap();
        state.exit_pending = true;
        state.cancel_exit();
        until(&mut state, Phase::Closed);
        assert!(
            !state.exit_pending,
            "a completed stop must not revive a cancelled quit"
        );
        assert!(state.runtime.process().is_none());
        assert!(state.runtime.error().is_none());
    }
}

impl Process for Terminal {
    fn stop(&mut self) -> Result<(), String> {
        self.stop_for_restart()
    }
}

#[derive(Clone)]
struct FloatingLaunch {
    launch: Launch,
    cwd: std::path::PathBuf,
    preferred_shell: Option<String>,
    scrollback: usize,
    tmux_id: Option<String>,
}

impl FloatingLaunch {
    fn spawn(self, automatic: bool) -> Result<Terminal, String> {
        let channel = crate::ssh_reconnect::Channel::for_launch(&self.launch, automatic)?;
        let command = launch::pane_command(
            &self.launch,
            &self.cwd,
            None,
            self.preferred_shell.as_deref(),
            self.tmux_id.as_deref(),
            channel.as_ref(),
        )?;
        let mut terminal = Terminal::spawn_command_with_scrollback(command, self.scrollback)
            .map_err(|error| error.to_string())?;
        terminal.ssh_channel = channel;
        Ok(terminal)
    }
}

pub(super) struct State {
    pub ui: FloatingTerminalState,
    runtime: Controller<Terminal>,
    title: String,
    pending_launch: Option<FloatingLaunch>,
    active_launch: Option<FloatingLaunch>,
    ssh_retry: crate::ssh_reconnect::Retry,
    toggle_id: Option<egui::Id>,
    pub(super) exit_pending: bool,
    pub(super) exit_failed: bool,
    pub(super) force_exit: bool,
}
impl State {
    pub fn new(ctx: &egui::Context, saved: workspace::FloatingPreferences) -> Self {
        let ctx = ctx.clone();
        Self::new_with_repaint(move || ctx.request_repaint(), saved)
    }
    pub fn new_with_repaint(
        repaint: impl Fn() + Send + Sync + 'static,
        saved: workspace::FloatingPreferences,
    ) -> Self {
        Self {
            ui: FloatingTerminalState::from_saved(saved.open, saved.height),
            runtime: Controller::new(repaint),
            title: "Terminal".into(),
            pending_launch: None,
            active_launch: None,
            ssh_retry: Default::default(),
            toggle_id: None,
            exit_pending: false,
            exit_failed: false,
            force_exit: false,
        }
    }
    pub fn busy(&self) -> bool {
        self.runtime.phase() != Phase::Closed
    }
    pub fn process_id(&self) -> Option<u32> {
        self.runtime.process_ref().and_then(Terminal::process_id)
    }
    #[cfg(feature = "visual-test")]
    pub fn capture_ready(&self) -> bool {
        self.runtime.phase() == Phase::Ready || self.runtime.error().is_some()
    }
    pub fn cancel_exit(&mut self) {
        self.exit_pending = false;
        self.exit_failed = false;
        self.force_exit = false;
    }
    pub fn terminal_focused(&mut self, ctx: &egui::Context) -> bool {
        self.ui.is_open()
            && self
                .runtime
                .process()
                .is_some_and(|terminal| terminal.has_focus(ctx))
    }
    pub fn paste_text(&mut self, text: &str) -> bool {
        if let Some(terminal) = self.runtime.process()
            && !terminal.exited()
        {
            terminal.paste_text(text);
            true
        } else {
            false
        }
    }
    pub fn run_quick_command(&mut self, text: &str, press_enter: bool) -> bool {
        if let Some(terminal) = self.runtime.process()
            && !terminal.exited()
        {
            terminal.run_quick_command(text, press_enter).is_ok()
        } else {
            false
        }
    }
    pub fn poll(&mut self) {
        let before = self.runtime.phase();
        self.runtime.poll();
        if self.runtime.phase() == Phase::Closed {
            self.active_launch = None;
            self.pending_launch = None;
            self.ssh_retry = Default::default();
            return;
        }
        if before != Phase::Ready && self.runtime.phase() == Phase::Ready {
            if self.runtime.error().is_some() {
                self.ui.show();
                self.pending_launch = None;
                self.ssh_retry.give_up();
            } else if let Some(plan) = self.pending_launch.take() {
                self.title = match &plan.launch {
                    Launch::Ssh(host) => format!("SSH · {}", host.alias),
                    _ => "Terminal".into(),
                };
                self.active_launch = Some(plan);
            }
        }
        let can_retry = !self.exit_pending && !self.exit_failed && self.runtime.error().is_none();
        let Some(terminal) = self.runtime.process() else {
            return;
        };
        terminal.poll();
        if can_retry
            && let Some(plan) = &self.active_launch
            && let Some(channel) = &terminal.ssh_channel
            && self
                .ssh_retry
                .due(std::time::Instant::now(), channel, terminal.exit_code())
        {
            let plan = plan.clone();
            // Restart the original host even after the user switches projects.
            // Worker admission failures leave the process in place; the next poll
            // schedules another bounded attempt.
            let _ = self.runtime.restart(move || plan.spawn(true));
        }
    }
    #[cfg(all(test, feature = "visual-test"))]
    pub fn install_find_fixture(&mut self, terminal: Terminal) {
        self.runtime = Controller::new(|| {});
        self.runtime.open(move || Ok(terminal)).unwrap();
        for _ in 0..1000 {
            self.runtime.poll();
            if self.runtime.process_ref().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(self.runtime.process_ref().is_some());
        self.ui.show();
    }
    pub fn process_ref(&self) -> Option<&Terminal> {
        self.runtime.process_ref()
    }
    pub fn process_mut(&mut self) -> Option<&mut Terminal> {
        self.runtime.process()
    }
    pub fn phase(&self) -> Phase {
        self.runtime.phase()
    }
    pub fn title_str(&self) -> &str {
        &self.title
    }
    pub fn status_notice(&self, french: bool) -> Option<&str> {
        self.runtime.error().or_else(|| {
            self.ssh_retry.notice(
                french,
                self.runtime
                    .process_ref()
                    .is_some_and(Terminal::ssh_transport_lost),
            )
        })
    }
}

fn refused_text(error: Refused) -> String {
    crate::i18n::tr(match error {
        Refused::AlreadyOpen => "The floating terminal is already open.",
        Refused::NotOpen => "The floating terminal is not open.",
        Refused::Busy => "A floating terminal operation is still in progress.",
        Refused::Capacity => "Terminal workers are busy. Try again when an operation finishes.",
        Refused::NoThread => "Cannot start the terminal worker. Try again.",
    })
    .into()
}

impl Tessel {
    #[cfg(feature = "visual-test")]
    pub(super) fn floating_fixture(&mut self) {
        self.floating.ui.show();
        self.floating.title = "Terminal".into();
        if let Err(error) = self
            .floating
            .runtime
            .open(|| Terminal::visual_fixture().map_err(|error| error.to_string()))
        {
            self.error = format!("Floating fixture: {error:?}");
        }
    }
    pub(super) fn drain_floating_for_exit(&mut self) -> bool {
        if self.floating.force_exit || !self.floating.busy() {
            return true;
        }
        if !self.floating.exit_pending {
            match self.floating.runtime.close() {
                Ok(()) => {
                    self.floating.exit_pending = true;
                    self.floating.exit_failed = false;
                }
                Err(error) => {
                    self.floating.exit_failed = true;
                    self.error = refused_text(error);
                }
            }
        }
        false
    }
    pub(super) fn poll_floating(&mut self, ctx: &egui::Context) {
        self.floating.poll();
        if self.floating.exit_pending {
            if self.floating.runtime.phase() == Phase::Closed {
                self.floating.exit_pending = false;
                if !self.exit_background_busy() {
                    self.finish_exit(ctx);
                }
            } else if self.floating.runtime.phase() == Phase::Ready {
                self.floating.exit_pending = false;
                self.floating.exit_failed = true;
                self.error = crate::i18n::tr(
                    self.floating
                        .runtime
                        .error()
                        .unwrap_or("Could not stop the floating terminal."),
                )
                .into();
            }
        }
    }
    fn floating_launch(&self) -> Result<(Launch, std::path::PathBuf), String> {
        let cwd = self
            .state
            .active()
            .map(|p| p.path.clone())
            .unwrap_or_else(launch::default_directory);
        let launch = if self.view == View::Remote {
            Launch::Ssh(
                self.remote
                    .terminal_host(&self.state.hosts)
                    .ok_or("Select a remote host first.")?,
            )
        } else {
            Launch::Shell
        };
        Ok((launch, cwd))
    }
    fn start_floating(&mut self, restart: bool) {
        if !cfg!(windows) {
            self.error = crate::i18n::tr("Floating terminals require verified process-group shutdown, currently available on Windows only.").into();
            return;
        }
        let (launch, cwd) = match self.floating_launch() {
            Ok(value) => value,
            Err(error) => {
                self.error = error;
                return;
            }
        };
        let mut tmux_id = self.floating.active_launch.as_ref()
            .filter(|previous| matches!((&launch, &previous.launch),
                (Launch::Ssh(next), Launch::Ssh(old)) if next.alias == old.alias && next.folder == old.folder))
            .and_then(|previous| previous.tmux_id.clone());
        if let Err(error) = launch::prepare_ssh_tmux_id(&launch, &mut tmux_id) {
            self.error = error;
            return;
        }
        let preferred_shell = matches!(launch, Launch::Shell)
            .then(|| self.state.launcher_prefs.preferred_shell.clone())
            .flatten();
        let plan = FloatingLaunch {
            launch,
            cwd,
            preferred_shell,
            tmux_id,
            scrollback: self.state.terminal_prefs.scrollback,
        };
        let target = plan.clone();
        let make = move || target.spawn(false);
        let result = if restart {
            self.floating.runtime.restart(make)
        } else {
            self.floating.runtime.open(make)
        };
        match result {
            Ok(()) => {
                self.floating.pending_launch = Some(plan);
                self.floating.ssh_retry = Default::default();
            }
            Err(error) => self.error = refused_text(error),
        }
    }
    fn floating_action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Show => {
                self.floating.ui.show();
                if !self.floating.busy() {
                    self.start_floating(false);
                }
            }
            Action::Hide => {
                self.floating.ui.hide();
            }
            Action::Close if self.floating.runtime.phase() == Phase::Closed => {
                self.floating.ui.close()
            }
            Action::Close => match self.floating.runtime.close() {
                Ok(()) => self.floating.ui.close(),
                Err(error) => {
                    self.floating.ui.show();
                    self.error = refused_text(error);
                }
            },
            Action::Restart => match self.floating.runtime.phase() {
                Phase::Closed => self.start_floating(false),
                Phase::Ready => self.start_floating(true),
                _ => {}
            },
            Action::CopyOutput => {
                if let Some(terminal) = self.floating.runtime.process() {
                    ctx.copy_text(terminal.output());
                }
            }
            Action::HeightChanged(height) => {
                self.floating.ui.set_height(height);
            }
            Action::FocusTerminal => {} // The panel owns the stable widget's focus.
            Action::RestoreFocus(id) => {
                if let Some(id) = id {
                    ctx.memory_mut(|memory| memory.request_focus(id));
                } else if self.view == View::Work
                    && let Some(terminal) = self
                        .sessions
                        .iter()
                        .find(|session| {
                            Some(session.id) == self.active_session
                                && session.project == self.state.active_project
                        })
                        .and_then(|session| session.terminal.as_ref())
                {
                    terminal.request_focus(ctx);
                }
            }
        }
    }
    pub(super) fn floating_shortcuts(&mut self, ctx: &egui::Context, blocked: bool) {
        let typing = ctx.wants_keyboard_input() && !self.floating.ui.has_keyboard();
        for action in Panel::handle_shortcuts(ctx, &mut self.floating.ui, blocked, typing) {
            self.floating_action(ctx, action);
        }
    }
    pub(super) fn floating_toolbar(&mut self, ui: &mut egui::Ui) {
        let response = Panel::toolbar_button(
            ui,
            self.floating.ui.is_open(),
            self.state.language == crate::i18n::Language::French,
        );
        self.floating.toggle_id = Some(response.id);
        if response.clicked() {
            self.toggle_floating(ui.ctx());
        }
    }
    pub(super) fn toggle_floating(&mut self, ctx: &egui::Context) {
        let action = if self.floating.ui.is_open() {
            Action::Hide
        } else {
            Action::Show
        };
        self.floating_action(ctx, action);
    }
    pub(super) fn toggle_floating_headless(&mut self) {
        if self.floating.ui.is_open() {
            self.floating.ui.hide();
        } else {
            self.floating.ui.show();
            if !self.floating.busy() {
                self.start_floating(false);
            }
        }
        self.save();
    }
    pub(super) fn restart_floating_headless(&mut self) {
        match self.floating.runtime.phase() {
            Phase::Closed => self.start_floating(false),
            Phase::Ready => self.start_floating(true),
            _ => {}
        }
    }
    pub(super) fn close_floating_headless(&mut self) {
        if self.floating.runtime.phase() == Phase::Closed {
            self.floating.ui.close();
        } else {
            match self.floating.runtime.close() {
                Ok(()) => self.floating.ui.close(),
                Err(error) => {
                    self.floating.ui.show();
                    self.error = refused_text(error);
                }
            }
        }
        self.save();
    }
    pub(super) fn set_floating_height_headless(&mut self, height: f32) {
        self.floating.ui.set_height(f64::from(height));
        self.state.floating.height = self.floating.ui.height;
        self.save();
    }
    pub(super) fn show_floating(
        &mut self,
        ctx: &egui::Context,
        rect: egui::Rect,
    ) -> Option<egui::Rect> {
        let blocked = self.overlay.is_some()
            || self.command_palette.open
            || self.finder.is_open()
            || self.native_modal_open()
            || self.pending_exit
            || self.pending_close.is_some()
            || self.pending_close_session.is_some()
            || self.pending_remove_project.is_some()
            || self.file_action.is_some()
            || self.restore_preview.is_some()
            || self.toolbar_menu_open
            || self.status_bar.has_open_popover();
        let phase = self.floating.runtime.phase();
        let error = self
            .floating
            .status_notice(self.state.language == crate::i18n::Language::French)
            .map(str::to_owned);
        let focus_id = egui::Id::new("floating-terminal-content");
        let input = FloatingTerminalInput {
            workspace_rect: rect,
            title: &self.floating.title,
            has_terminal: phase == Phase::Ready,
            starting: matches!(phase, Phase::Opening | Phase::Restarting | Phase::Closing),
            can_restart: matches!(phase, Phase::Ready | Phase::Closed),
            has_output: phase == Phase::Ready,
            dialog_open: blocked,
            typing_in_field: ctx.wants_keyboard_input() && !self.floating.ui.has_keyboard(),
            french: self.state.language == crate::i18n::Language::French,
            palette: crate::theme::active(ctx).spec().ui,
            content_focus_id: Some(focus_id),
            toggle_button_id: self.floating.toggle_id,
        };
        let runtime = &mut self.floating.runtime;
        let terminal_prefs = self.state.terminal_prefs;
        let mut start = false;
        let output = Panel::show(
            ctx,
            "floating-terminal",
            &mut self.floating.ui,
            input,
            |ui, _body| {
                ui.add_enabled_ui(!blocked && !egui::Popup::is_any_open(ctx), |ui| {
                    if let Some(error) = &error {
                        ui.colored_label(egui::Color32::LIGHT_RED, crate::i18n::tr(error));
                    }
                    if let Some(terminal) = runtime.process() {
                        terminal.show_with_prefs(
                            ui,
                            self.state.font_size,
                            &terminal_prefs,
                            Some(focus_id),
                        );
                    } else if phase == Phase::Closed {
                        start = ui.button(crate::i18n::tr("Open terminal")).clicked();
                    } else {
                        ui.spinner();
                    }
                });
            },
        );
        for action in output.actions {
            self.floating_action(ctx, action);
        }
        if start {
            self.start_floating(false);
        }
        let saved = workspace::FloatingPreferences {
            open: self.floating.ui.is_open(),
            height: self.floating.ui.height,
        };
        if saved != self.state.floating {
            self.state.floating = saved;
            self.save();
        }
        output.panel_rect
    }
}
