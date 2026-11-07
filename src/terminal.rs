mod backend;
pub mod host_apply;
pub mod host_commands;
pub mod host_emulator;
pub mod host_events;
pub mod host_handoff;
pub mod host_history;
pub mod host_lease_desk;
pub mod host_lease_keeper;
pub mod host_opening;
pub mod host_runtime;
pub mod host_terminal_list;
pub mod host_viewport;
pub mod host_worker;
mod local;
mod mouse;
#[cfg(windows)]
mod native;
use crate::i18n::tr;
use eframe::egui;
use portable_pty::CommandBuilder;
#[cfg(test)]
use std::sync::mpsc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BroadcastInput {
    Raw(Vec<u8>),
    Paste(String),
    CursorKey(u8),
}

pub struct Terminal {
    backend: backend::Backend,
    parser: vt100::Parser,
    model_dirty: bool,
    observed_model: Option<crate::pane_session_model::ShownAgentModel>,
    model_catalog: Vec<serde_json::Value>,
    model_agent: String,
    model_profile: Option<crate::pane_model_profile::Profile>,
    model_runtime: crate::pane_model_runtime::Runtime,
    model_screen_present: bool,
    output_revision: u64,
    last_output_at: Option<std::time::Instant>,
    size: (u16, u16),
    scrollback_limit: usize,
    status: String,
    exited: bool,
    query_tail: Vec<u8>,
    selection: Option<Selection>,
    composition: String,
    composing: bool,
    mouse: mouse::Mouse,
    widget_id: Option<egui::Id>,
    broadcast_outbox: Vec<BroadcastInput>,
    agent_kind: Option<String>,
    find_open: bool,
    find_query: String,
    find_index: usize,
    find_focus_pending: bool,
    pending_multiline_paste: Option<String>,
    shift_context_requested: Option<egui::Pos2>,
    #[cfg(windows)]
    native_panel: crate::host_terminal_panel::HostTerminalPanel,
    #[cfg(windows)]
    native_action: Option<crate::host_terminal_panel::HostTerminalAction>,
    #[cfg(feature = "visual-test")]
    fixture: bool,
    #[cfg(feature = "visual-test")]
    model_fixture: bool,
}

impl Terminal {
    /// Called on the restart worker. No replacement may start until this succeeds.
    pub fn stop_for_restart(&mut self) -> Result<(), String> {
        self.backend.stop_for_restart()?;
        self.exited = true;
        Ok(())
    }
    pub fn take_broadcast_inputs(&mut self) -> Vec<BroadcastInput> {
        std::mem::take(&mut self.broadcast_outbox)
    }

    pub fn set_agent_kind(&mut self, agent: Option<&str>) {
        match agent {
            Some(a) if self.agent_kind.as_deref() != Some(a) => {
                self.agent_kind = Some(a.to_owned());
            }
            None => {
                self.agent_kind = None;
            }
            _ => {}
        }
    }

    pub fn has_selection(&self) -> bool {
        self.selection
            .is_some_and(|s| s.start != s.end && !self.selection_text(s).is_empty())
    }

    pub fn take_selection(&mut self) -> Option<String> {
        let selection = self.selection.filter(|s| s.start != s.end)?;
        let text = self.selection_text(selection);
        if text.is_empty() {
            return None;
        }
        self.selection = None;
        Some(text)
    }

    pub fn copy_selection(&mut self, ctx: &egui::Context) -> bool {
        if let Some(text) = self.take_selection() {
            ctx.copy_text(text);
            true
        } else {
            false
        }
    }

    pub fn paste_text(&mut self, text: &str) {
        if !self.accepts_broadcast_input() || text.is_empty() {
            return;
        }
        if text.len() > 65536 {
            self.status = "Paste limit: 64 KiB".into();
            return;
        }
        if !crate::layout_broadcast_ui::is_terminal_reply(text.as_bytes()) {
            self.broadcast_outbox
                .push(BroadcastInput::Paste(text.to_owned()));
        }
        let bytes = if self.parser.screen().bracketed_paste() {
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.as_bytes().to_vec()
        };
        self.back_to_live();
        if let Err(error) = self.send(bytes) {
            self.status = error;
        }
    }

    pub fn paste_from_clipboard(&mut self) {
        if let Some(text) = read_system_clipboard() {
            self.paste_text(&text);
        }
    }

    #[must_use]
    pub fn screen_contains(&self, needle: &str) -> bool {
        !needle.is_empty() && self.parser.screen().contents().contains(needle)
    }

    pub fn send_installer_line(&mut self, line: &str) -> Result<(), String> {
        if !self.accepts_broadcast_input() || line.is_empty() {
            return Err("Terminal is not ready for input.".into());
        }
        if line.len() > 65536 {
            return Err("Installer command exceeds 64 KiB.".into());
        }
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\r');
        self.back_to_live();
        self.send(bytes)
    }

    pub fn run_quick_command(&mut self, text: &str, press_enter: bool) -> Result<(), String> {
        if !self.accepts_broadcast_input() || text.is_empty() {
            return Err("Terminal is not ready for input.".into());
        }
        if text.len() > 65536 {
            return Err("Quick command exceeds 64 KiB.".into());
        }
        if press_enter {
            let mut bytes = Vec::with_capacity(text.len() + 1);
            bytes.extend_from_slice(text.as_bytes());
            bytes.push(b'\r');
            self.back_to_live();
            self.send(bytes)
        } else {
            self.paste_text(text);
            Ok(())
        }
    }

    pub fn clear_screen(&mut self) {
        self.selection = None;
        self.parser.set_scrollback(0);
        self.parser.process(b"\x1b[2J\x1b[3J\x1b[H");
        self.model_dirty = true;
    }

    pub fn reset_modes(&mut self) {
        let release = self
            .mouse
            .release(self.parser.screen().mouse_protocol_encoding());
        if self.backend.writable() && !release.is_empty() {
            let _ = self.send(release);
        }
        self.parser.process(
            b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?2004l\x1b[?1l\x1b[?25h\x1b[0m",
        );
        self.composing = false;
        self.composition.clear();
        self.selection = None;
        self.back_to_live();
    }

    pub fn open_find(&mut self) {
        self.find_open = true;
        self.find_focus_pending = true;
    }

    pub fn is_find_open(&self) -> bool {
        self.find_open
    }

    pub fn take_context_menu_request(&mut self) -> Option<egui::Pos2> {
        self.shift_context_requested.take()
    }

    fn search_matches(&mut self, needle: &str) -> Vec<(usize, u16, u16, u16)> {
        if needle.is_empty() {
            return Vec::new();
        }
        let needle_lower = needle.to_lowercase();
        let needle_chars = needle_lower.chars().count().max(1) as u16;
        let saved_sb = self.parser.screen().scrollback();
        self.parser.set_scrollback(usize::MAX);
        let max_sb = self.parser.screen().scrollback();
        let (rows, cols) = self.parser.screen().size();
        let total_lines = max_sb + usize::from(rows);
        let mut hits = Vec::new();
        for abs_line in 0..total_lines {
            // To view `abs_line` at row 0 when possible:
            let sb = max_sb.saturating_sub(abs_line);
            let row = if abs_line <= max_sb {
                0
            } else {
                (abs_line - max_sb) as u16
            };
            if row >= rows {
                continue;
            }
            self.parser.set_scrollback(sb);
            let line = self.parser.screen().contents_between(row, 0, row, cols);
            if line.is_empty() {
                continue;
            }
            let lower = line.to_lowercase();
            let mut byte_from = 0usize;
            while let Some(rel) = lower[byte_from..].find(&needle_lower) {
                let match_byte = byte_from + rel;
                let start_col = (lower[..match_byte].chars().count() as u16).min(cols);
                let end_col = start_col.saturating_add(needle_chars).min(cols);
                hits.push((sb, row, start_col, end_col));
                byte_from =
                    match_byte + lower[match_byte..].chars().next().map_or(1, char::len_utf8);
            }
        }
        self.parser.set_scrollback(saved_sb);
        hits
    }

    fn jump_to_find_match(&mut self, step: isize) -> usize {
        let hits = self.search_matches(&self.find_query.clone());
        if hits.is_empty() {
            self.find_index = 0;
            return 0;
        }
        let len = hits.len();
        if step == 0 {
            self.find_index = self.find_index.min(len - 1);
        } else if step > 0 {
            self.find_index = (self.find_index + 1) % len;
        } else {
            self.find_index = (self.find_index + len - 1) % len;
        }
        let (sb, row, start_col, end_col) = hits[self.find_index];
        self.parser.set_scrollback(sb);
        self.selection = Some(Selection {
            start: (row, start_col),
            end: (row, end_col),
        });
        len
    }

    pub fn accepts_broadcast_input(&self) -> bool {
        #[cfg(windows)]
        if matches!(self.backend, backend::Backend::Native(_))
            && self.native_panel.blocks_terminal_focus()
        {
            return false;
        }
        self.backend.writable() && !self.exited && !self.viewing_archive()
    }

    pub fn send_broadcast(&mut self, input: &BroadcastInput) {
        if !self.accepts_broadcast_input() {
            return;
        }
        let bytes = match input {
            BroadcastInput::Raw(raw) => {
                if raw.is_empty() || crate::layout_broadcast_ui::is_terminal_reply(raw) {
                    return;
                }
                raw.clone()
            }
            BroadcastInput::Paste(text) => {
                if text.is_empty()
                    || text.len() > 65536
                    || crate::layout_broadcast_ui::is_terminal_reply(text.as_bytes())
                {
                    return;
                }
                if self.parser.screen().bracketed_paste() {
                    format!("\x1b[200~{text}\x1b[201~").into_bytes()
                } else {
                    text.as_bytes().to_vec()
                }
            }
            BroadcastInput::CursorKey(final_byte) => {
                let prefix = if self.parser.screen().application_cursor() {
                    b'O'
                } else {
                    b'['
                };
                vec![27, prefix, *final_byte]
            }
        };
        self.selection = None;
        self.back_to_live();
        if let Err(error) = self.send(bytes) {
            self.status = error;
        }
    }
    pub fn exited(&self) -> bool {
        self.exited
    }
    pub fn process_id(&self) -> Option<u32> {
        (!self.exited).then(|| self.backend.process_id()).flatten()
    }
    pub fn active_within(&self, window: std::time::Duration) -> bool {
        !self.exited && self.last_output_at.is_some_and(|at| at.elapsed() <= window)
    }
    #[cfg(windows)]
    pub fn take_native_action(&mut self) -> Option<crate::host_terminal_panel::HostTerminalAction> {
        self.native_action.take()
    }

    #[cfg(windows)]
    pub fn native_identity(&self) -> Option<crate::workspace::SavedHostTerminalIdentity> {
        let backend::Backend::Native(native) = &self.backend else {
            return None;
        };
        native
            .slot
            .as_ref()
            .map(|slot| crate::workspace::SavedHostTerminalIdentity {
                epoch: slot.epoch.clone(),
                terminal: slot.terminal.clone(),
            })
    }

    pub fn native_header_tag(&self) -> Option<crate::host_terminal_panel::HostHeaderTag> {
        #[cfg(windows)]
        if let backend::Backend::Native(native) = &self.backend {
            return crate::host_terminal_panel::HostTerminalPanel::header_tag(&native.phase());
        }
        None
    }

    #[cfg(windows)]
    pub fn native_owns_escape(&self) -> bool {
        matches!(self.backend, backend::Backend::Native(_))
            && (self.native_panel.has_pending_close_choice() || self.native_panel.is_escape_held())
    }

    /// Whether the app can remove this pane now. A controlled remote terminal
    /// offers explicit detach/terminate choices; an opening is cancelled first.
    #[cfg(windows)]
    pub fn request_native_close(&mut self) -> bool {
        use crate::host_terminal_panel::HostTerminalAction as Action;
        let backend::Backend::Native(native) = &mut self.backend else {
            return true;
        };
        match self
            .native_panel
            .request_close(&native.phase(), native.capabilities())
        {
            Some(Action::Detach) => !native.cancel_opening(&mut self.status),
            Some(Action::ClosePane) => true,
            Some(action) => {
                self.native_action = Some(action);
                false
            }
            None => false,
        }
    }

    #[cfg(windows)]
    pub fn native_cancelled(&self) -> bool {
        matches!(&self.backend, backend::Backend::Native(native)
            if matches!(native.failure, Some(host_opening::Failure::Cancelled)))
    }

    #[cfg(windows)]
    fn native_overlays(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        response: &egui::Response,
    ) -> bool {
        use crate::host_terminal_panel::{HostTerminalAction as Action, HostTerminalInput};
        let backend::Backend::Native(native) = &mut self.backend else {
            return true;
        };
        // A global dialog owns keyboard and painting while this pane is disabled.
        // In particular, a pane's pending close prompt must not consume its Escape.
        if !ui.is_enabled() {
            self.native_panel.release_key_latches(ui.ctx());
            native.allow_replies = false;
            return false;
        }
        let phase = native.phase();
        let epoch = native.host.client.epoch();
        let persistence = native.host.persistence().survives_logout;
        let input = HostTerminalInput {
            id_salt: response.id.with("host-overlay"),
            host_label: &native.alias,
            epoch: epoch.as_ref().map(|e| e.as_str()),
            phase: &phase,
            capabilities: native.capabilities(),
            terminal_focused: response.has_focus() && ui.is_enabled(),
            lease_save: native.lease_access(),
            persistence_warning: epoch.as_ref().and_then(|epoch| {
                use crate::host_terminal_panel::{PersistenceWarningInput, SurvivesLogout};
                use tessel_host_protocol::Survival;
                match persistence {
                    Survival::Yes => None,
                    Survival::No => Some(PersistenceWarningInput::new(
                        epoch.as_str(),
                        SurvivesLogout::No,
                    )),
                    Survival::Unknown => Some(PersistenceWarningInput::new(
                        epoch.as_str(),
                        SurvivesLogout::Unknown,
                    )),
                }
            }),
            skipped_output: native
                .feed
                .as_ref()
                .filter(|feed| feed.resynchronized)
                .and(native.slot.as_ref())
                .map(|slot| {
                    crate::host_terminal_panel::SkippedOutputInput::resynchronized(
                        slot.epoch.as_str(),
                    )
                }),
            scrollback_lines: native.scroll.total_scrollback(&self.parser),
            french: crate::i18n::is_french(),
        };
        let layer = egui::LayerId::new(ui.layer_id().order, response.id.with("host-overlay-layer"));
        ui.ctx().set_sublayer(ui.layer_id(), layer);
        let mut overlay = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(response.id.with("host-overlay-ui"))
                .layer_id(layer)
                .max_rect(rect),
        );
        overlay.set_clip_rect(ui.clip_rect().intersect(rect));
        let action = self
            .native_panel
            .intercept_key(ui.ctx(), &input)
            .or_else(|| self.native_panel.show_overlays(&mut overlay, rect, &input));
        let allows = ui.is_enabled() && self.native_panel.accepts_input(&phase);
        // Replies always describe the live parser, never the archived cells.
        native.allow_replies = allows;
        match action {
            Some(Action::RetryOpening) => {
                native.retry_opening();
            }
            Some(Action::RetrySaveLease) => native.retry_save(),
            Some(Action::TakeControl) => {
                if let Some(control) = &native.control {
                    let _ = control.take_control();
                }
            }
            Some(Action::Terminate) => {
                if let Some(control) = &native.control {
                    let _ = control.terminate();
                }
            }
            Some(Action::Detach) if native.cancel_opening(&mut self.status) => {}
            Some(Action::BackToLiveOutput) => {
                native.scroll.reset_to_live(&mut self.parser);
                self.selection = None;
                if self.status == tr("History: return to live output to type.") {
                    self.status.clear();
                }
            }
            Some(action) => self.native_action = Some(action),
            None => {}
        }
        allows
    }
    fn selection_text(&self, selection: Selection) -> String {
        self.viewport().select_text(selection.start, selection.end)
    }

    fn visible_text(&self) -> String {
        self.viewport().contents()
    }

    fn viewport(&self) -> host_viewport::Viewport<'_> {
        #[cfg(windows)]
        if let backend::Backend::Native(native) = &self.backend {
            return host_viewport::Viewport::new(
                native.feed.as_ref().map(|feed| &feed.history),
                self.parser.screen(),
                &native.scroll,
            );
        }
        host_viewport::Viewport::new(None, self.parser.screen(), &Default::default())
    }

    fn viewing_archive(&self) -> bool {
        #[cfg(windows)]
        if let backend::Backend::Native(native) = &self.backend {
            return native.scroll.in_archived_history();
        }
        false
    }

    fn viewing_scrollback(&self) -> bool {
        self.viewing_archive() || self.parser.screen().scrollback() > 0
    }

    fn back_to_live(&mut self) {
        #[cfg(windows)]
        if let backend::Backend::Native(native) = &mut self.backend {
            native.scroll.reset_to_live(&mut self.parser);
        }
        self.parser.set_scrollback(0);
        self.selection = None;
        if self.status == tr("History: return to live output to type.") {
            self.status.clear();
        }
    }

    fn scroll_by(&mut self, lines: i64) {
        self.selection = None;
        #[cfg(windows)]
        if let backend::Backend::Native(native) = &mut self.backend {
            native.scroll.scroll_by(
                lines,
                native.feed.as_ref().map(|feed| &feed.history),
                &mut self.parser,
            );
            return;
        }
        let scroll = (self.parser.screen().scrollback() as i64).saturating_add(lines);
        let max_scroll = (self.scrollback_limit.clamp(500, 100_000)) as i64;
        self.parser
            .set_scrollback(scroll.clamp(0, max_scroll) as usize);
    }
    pub fn output(&self) -> String {
        self.parser.screen().contents()
    }
    #[cfg(feature = "visual-test")]
    pub fn enable_model_fixture(&mut self) {
        self.model_fixture = true;
        let rows = self.size.0;
        self.process_output(format!("\x1b[{};1H\x1b[2K› Ask Codex anything\x1b[{};1H\x1b[2K\x1b[{};1H\x1b[2KGPT-5.6 Sol xhigh fast", rows - 2, rows - 1, rows).as_bytes());
    }
    #[cfg(feature = "visual-test")]
    pub fn enable_model_file_fixture(&mut self, root: &std::path::Path, cwd: &std::path::Path) {
        let mut command = CommandBuilder::new("synthetic-model-fixture");
        command.env_clear();
        command.cwd(cwd);
        command.env("CODEX_HOME", root);
        self.model_profile = crate::pane_model_profile::Profile::capture("codex", &command, cwd);
    }
    pub fn observed_model(
        &mut self,
        agent: &str,
        models: &[serde_json::Value],
    ) -> Option<&crate::pane_session_model::ShownAgentModel> {
        if self
            .model_profile()
            .is_some_and(|profile| profile.agent() != agent)
        {
            return None;
        }
        if self.model_agent != agent {
            self.observed_model = None;
            self.model_screen_present = false;
            self.model_agent = agent.to_owned();
            self.model_dirty = true;
        }
        if !crate::terminal_model::supported(agent) {
            return None;
        }
        if self.model_dirty || self.model_catalog != models {
            self.model_dirty = false;
            if self.model_catalog != models {
                self.model_catalog = models.to_vec();
            }
            let lines = crate::terminal_model::live_footer(&mut self.parser);
            let seen = crate::terminal_model::observe(agent, &lines, models);
            self.model_screen_present = seen.is_some();
            if let Some(seen) = seen {
                self.observed_model = Some(seen);
            }
        }
        self.observed_model.as_ref()
    }
    pub fn header_model(
        &mut self,
        agent: &str,
        models: &[serde_json::Value],
        conversation: Option<&str>,
        ctx: &egui::Context,
    ) -> Option<crate::pane_session_model::ShownAgentModel> {
        let screen = self.observed_model(agent, models).cloned();
        let wake = ctx.clone();
        let session = self
            .model_runtime
            .tick(
                self.model_profile.as_ref().filter(|p| p.agent() == agent),
                conversation,
                self.output_revision,
                std::time::Instant::now(),
                move || wake.request_repaint(),
            )
            .cloned();
        if self.model_screen_present {
            screen
        } else {
            session.or(screen)
        }
    }
    pub fn request_focus(&self, ctx: &egui::Context) {
        if let Some(id) = self.widget_id {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }
    pub fn has_focus(&self, ctx: &egui::Context) -> bool {
        self.widget_id
            .is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
    }
    #[cfg(feature = "visual-test")]
    pub fn visual_fixture() -> Result<Self, Box<dyn std::error::Error>> {
        let command = crate::launch::command(
            &crate::launch::Launch::Shell,
            &crate::launch::default_directory(),
        )?;
        let mut terminal = Self::spawn_command(command)?;
        terminal.process_output("\x1b[2J\x1b[HTessel Rust terminal fixture\r\nFrançais : Québec, été, déjà.\r\nWide cells: A🙂B🙂C\r\n\x1b[44m Blue background 🙂 continues \x1b[0m\r\n\x1b[4mUnderlined text and 🙂\x1b[0m\r\n\x1b[32mGreen foreground\x1b[0m\r\nCtrl+A-Z / Shift+Tab / F1-F12 / modified navigation".as_bytes());
        terminal.fixture = true;
        Ok(terminal)
    }
    #[cfg(test)]
    pub fn spawn() -> Result<Self, Box<dyn std::error::Error>> {
        let command = crate::launch::command(
            &crate::launch::Launch::Shell,
            &crate::launch::default_directory(),
        )?;
        Self::spawn_command(command)
    }

    pub fn model_profile(&self) -> Option<&crate::pane_model_profile::Profile> {
        self.model_profile.as_ref()
    }

    /// For a known provider launch, on its launch worker. Custom shell wrappers
    /// use spawn_command because their inner environment is not observable here.
    pub fn spawn_agent_command(
        agent: &str,
        command: CommandBuilder,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::spawn_agent_command_with_scrollback(agent, command, 5000)
    }

    pub fn spawn_agent_command_with_scrollback(
        agent: &str,
        command: CommandBuilder,
        scrollback: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let profile = directories::BaseDirs::new().and_then(|dirs| {
            crate::pane_model_profile::Profile::capture(agent, &command, dirs.home_dir())
        });
        let mut terminal = Self::spawn_command_with_scrollback(command, scrollback)?;
        terminal.model_profile = profile;
        Ok(terminal)
    }

    pub fn spawn_command(command: CommandBuilder) -> Result<Self, Box<dyn std::error::Error>> {
        Self::spawn_command_with_scrollback(command, 5000)
    }

    pub fn spawn_command_with_scrollback(
        command: CommandBuilder,
        scrollback: usize,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self::with_backend_and_scrollback(
            backend::Backend::Local(local::LocalPty::spawn(command)?),
            scrollback,
        ))
    }

    #[cfg(windows)]
    pub fn spawn_native(
        host: crate::host_opening_adapter::HostConnection,
        target: host_opening::Target,
        runtime: &host_runtime::Runtime,
        context: egui::Context,
    ) -> Self {
        Self::with_backend_and_scrollback(
            backend::Backend::Native(Box::new(native::Native::open(
                host, target, runtime, context,
            ))),
            5000,
        )
    }

    #[cfg(test)]
    fn with_backend(backend: backend::Backend) -> Self {
        Self::with_backend_and_scrollback(backend, 5000)
    }

    fn with_backend_and_scrollback(backend: backend::Backend, scrollback: usize) -> Self {
        let scrollback = scrollback.clamp(500, 100_000);
        Self {
            backend,
            parser: vt100::Parser::new(30, 100, scrollback),
            model_dirty: true,
            observed_model: None,
            model_catalog: Vec::new(),
            model_agent: String::new(),
            model_profile: None,
            model_runtime: crate::pane_model_runtime::Runtime::default(),
            model_screen_present: false,
            output_revision: 0,
            last_output_at: None,
            size: (30, 100),
            scrollback_limit: scrollback,
            status: String::new(),
            exited: false,
            query_tail: Vec::new(),
            selection: None,
            composition: String::new(),
            composing: false,
            mouse: Default::default(),
            widget_id: None,
            broadcast_outbox: Vec::new(),
            agent_kind: None,
            find_open: false,
            find_query: String::new(),
            find_index: 0,
            find_focus_pending: false,
            pending_multiline_paste: None,
            shift_context_requested: None,
            #[cfg(windows)]
            native_panel: Default::default(),
            #[cfg(windows)]
            native_action: None,
            #[cfg(feature = "visual-test")]
            fixture: false,
            #[cfg(feature = "visual-test")]
            model_fixture: false,
        }
    }

    fn process_output(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.selection = None;
            self.model_dirty = true;
            self.output_revision = self.output_revision.wrapping_add(1);
            self.last_output_at = Some(std::time::Instant::now());
        }
        if let Some(error) = host_emulator::apply(
            &mut self.parser,
            &mut self.query_tail,
            bytes,
            true,
            |reply| self.backend.send(reply),
        ) {
            self.status = error;
        }
    }

    fn send(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.backend.send(bytes)
    }

    pub fn poll(&mut self) {
        #[cfg(windows)]
        if let backend::Backend::Native(native) = &mut self.backend {
            if native.poll(&mut self.parser, &mut self.query_tail, &mut self.status) {
                if native.selection_invalidated {
                    self.selection = None;
                }
                self.model_dirty = true;
                self.output_revision = self.output_revision.wrapping_add(1);
                self.last_output_at = Some(std::time::Instant::now());
            }
            self.size = self.parser.screen().size();
            self.exited = native
                .feed
                .as_ref()
                .is_some_and(|feed| matches!(feed.state, host_events::State::Exited { .. }));
            return;
        }
        #[cfg(feature = "visual-test")]
        if self.fixture {
            return;
        }
        for _ in 0..64 {
            let Some(bytes) = self.backend.try_output() else {
                break;
            };
            self.process_output(&bytes);
        }
        if let Some(exit) = self.backend.exit_status() {
            self.exited = true;
            self.status = format!("Shell exited: {exit}");
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, font_size: f32) {
        self.show_with_id(ui, font_size, None);
    }
    pub fn show_with_id(&mut self, ui: &mut egui::Ui, font_size: f32, id: Option<egui::Id>) {
        let legacy_prefs = crate::workspace::TerminalPreferences {
            confirm_multiline_paste: false,
            cursor_blink: false,
            ..Default::default()
        };
        self.show_with_prefs(ui, font_size, &legacy_prefs, id);
    }
    pub fn show_with_prefs(
        &mut self,
        ui: &mut egui::Ui,
        font_size: f32,
        prefs: &crate::workspace::TerminalPreferences,
        id: Option<egui::Id>,
    ) {
        self.broadcast_outbox.clear();
        let is_floating = id.is_some();
        if !self.status.is_empty() && matches!(self.backend, backend::Backend::Local(_)) {
            ui.colored_label(egui::Color32::LIGHT_RED, &self.status);
        }
        let mut refocus_terminal = false;
        if let Some(pending_text) = self.pending_multiline_paste.clone() {
            let french = crate::i18n::is_french();
            let line_count = pending_text.lines().count().max(2);
            let mut confirm_paste = false;
            let mut cancel_paste = false;
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    if french {
                        format!("Coller {line_count} lignes dans ce terminal ?")
                    } else {
                        format!("Paste {line_count} lines into this terminal?")
                    },
                );
                if ui
                    .button(if french {
                        "Coller quand même"
                    } else {
                        "Paste anyway"
                    })
                    .clicked()
                {
                    confirm_paste = true;
                }
                if ui
                    .button(if french { "Annuler" } else { "Cancel" })
                    .clicked()
                {
                    cancel_paste = true;
                }
            });
            if confirm_paste {
                self.pending_multiline_paste = None;
                self.paste_text(&pending_text);
                refocus_terminal = true;
            } else if cancel_paste {
                self.pending_multiline_paste = None;
                refocus_terminal = true;
            }
        }
        if self.find_open {
            let french = crate::i18n::is_french();
            let hint = if french {
                "Rechercher dans le terminal…"
            } else {
                "Find in terminal…"
            };
            let prev_tip = if french {
                "Précédent (Maj+Entrée)"
            } else {
                "Previous (Shift+Enter)"
            };
            let next_tip = if french {
                "Suivant (Entrée)"
            } else {
                "Next (Enter)"
            };
            let close_tip = if french {
                "Fermer (Échap)"
            } else {
                "Close (Esc)"
            };
            let mut close_find = false;
            let mut step_dir: Option<isize> = None;
            let total_matches = self.search_matches(&self.find_query.clone()).len();
            ui.horizontal(|ui| {
                let find_id = ui.id().with("terminal-find-input");
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.find_query)
                        .id(find_id)
                        .char_limit(256)
                        .desired_width((ui.available_width() - 128.0).max(80.0))
                        .hint_text(hint),
                );
                if self.find_focus_pending {
                    edit.request_focus();
                    if !ui.is_sizing_pass() {
                        self.find_focus_pending = false;
                    }
                }
                if edit.changed() {
                    self.find_index = 0;
                    step_dir = Some(0);
                }
                if edit.has_focus() || edit.lost_focus() {
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                        close_find = true;
                    } else if ui
                        .input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter))
                    {
                        step_dir = Some(-1);
                        edit.request_focus();
                    } else if ui
                        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    {
                        step_dir = Some(1);
                        edit.request_focus();
                    }
                }
                let badge = if self.find_query.is_empty() {
                    String::new()
                } else if total_matches == 0 {
                    "0/0".to_owned()
                } else {
                    format!(
                        "{}/{total_matches}",
                        self.find_index.min(total_matches - 1) + 1
                    )
                };
                if !badge.is_empty() {
                    ui.weak(badge);
                }
                if ui
                    .add_enabled(total_matches > 0, egui::Button::new("↑").small())
                    .on_hover_text(prev_tip)
                    .clicked()
                {
                    step_dir = Some(-1);
                }
                if ui
                    .add_enabled(total_matches > 0, egui::Button::new("↓").small())
                    .on_hover_text(next_tip)
                    .clicked()
                {
                    step_dir = Some(1);
                }
                if ui.small_button("×").on_hover_text(close_tip).clicked() {
                    close_find = true;
                }
            });
            if let Some(dir) = step_dir {
                let _ = self.jump_to_find_match(dir);
            }
            if close_find {
                self.find_open = false;
                refocus_terminal = true;
            }
        }
        let font = egui::FontId::monospace(font_size);
        let cell_width = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
        let line_scale = if prefs.line_height.is_finite() {
            prefs.line_height.clamp(1.0_f32, 3.0_f32)
        } else {
            1.0_f32
        };
        let line_height = ui.fonts_mut(|f| f.row_height(&font)) * line_scale;
        let available = ui.available_size();
        let cols = ((available.x / cell_width) as u16).clamp(20, 300);
        let rows = ((available.y / line_height) as u16).clamp(5, 150);
        #[cfg(windows)]
        let accepts_resize = match &self.backend {
            backend::Backend::Native(native) => {
                ui.is_enabled() && self.native_panel.accepts_resize(&native.phase())
            }
            backend::Backend::Local(_) => true,
        };
        #[cfg(not(windows))]
        let accepts_resize = true;
        if accepts_resize
            && self.size != (rows, cols)
            && matches!(self.backend.resize(rows, cols), Ok(true))
        {
            self.selection = None;
            self.parser.set_size(rows, cols);
            self.model_dirty = true;
            self.size = (rows, cols);
            #[cfg(feature = "visual-test")]
            if self.model_fixture {
                self.enable_model_fixture();
            }
        }
        let (rows, cols) = self.size;
        let (rect, response) = if let Some(id) = id {
            let (_, rect) = ui.allocate_space(available);
            (rect, ui.interact(rect, id, egui::Sense::click_and_drag()))
        } else {
            ui.allocate_exact_size(available, egui::Sense::click_and_drag())
        };
        self.widget_id = Some(response.id);
        #[cfg(windows)]
        let accepts_input =
            self.native_overlays(ui, rect, &response) && ui.is_enabled() && self.backend.writable();
        #[cfg(not(windows))]
        let accepts_input = ui.is_enabled() && self.backend.writable();
        #[cfg(windows)]
        let overlay_pointer = matches!(self.backend, backend::Backend::Native(_))
            && ui.input(|i| {
                i.pointer
                    .interact_pos()
                    .or_else(|| i.pointer.hover_pos())
                    .is_some_and(|p| self.native_panel.pointer_over_overlay(p))
            });
        #[cfg(not(windows))]
        let overlay_pointer = false;
        #[cfg(windows)]
        let focus_blocked = matches!(self.backend, backend::Backend::Native(_))
            && self.native_panel.blocks_terminal_focus();
        #[cfg(not(windows))]
        let focus_blocked = false;
        if ui.is_enabled()
            && !overlay_pointer
            && !focus_blocked
            && (refocus_terminal
                || response.clicked()
                || response.drag_started()
                || response.contains_pointer() && ui.input(|i| i.pointer.any_pressed())
                || prefs.focus_follows_mouse
                    && response.contains_pointer()
                    && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO))
        {
            response.request_focus();
        }
        if response.has_focus() && ui.is_enabled() {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        tab: accepts_input,
                        horizontal_arrows: accepts_input,
                        vertical_arrows: accepts_input,
                        escape: accepts_input,
                    },
                )
            });
        }
        #[cfg(feature = "visual-test")]
        if self.fixture {
            self.selection = Some(Selection {
                start: (1, 0),
                end: (1, 9),
            });
        }
        if response.lost_focus() || !accepts_input || self.viewing_archive() {
            self.composition.clear();
            self.composing = false;
        }
        let mode = if accepts_input && !overlay_pointer && !self.viewing_scrollback() {
            self.parser.screen().mouse_protocol_mode()
        } else {
            vt100::MouseProtocolMode::None
        };
        let shift = ui.input(|i| i.modifiers.shift);
        let tracking = mode != vt100::MouseProtocolMode::None;
        let scroll_requested = !overlay_pointer
            && !focus_blocked
            && (!tracking || shift)
            && response.hovered()
            && ui.input(|i| i.smooth_scroll_delta.y != 0.0);
        let release_mouse =
            !accepts_input || overlay_pointer || self.viewing_archive() || scroll_requested;
        let mouse_bytes = if release_mouse {
            self.mouse
                .release(self.parser.screen().mouse_protocol_encoding())
        } else {
            ui.input(|i| {
                self.mouse.events(
                    &i.events,
                    &mouse::Area {
                        rect,
                        cell: egui::vec2(cell_width, line_height),
                        size: self.size,
                        hovered: response.contains_pointer(),
                        pointer: i.pointer.hover_pos(),
                        modifiers: i.modifiers,
                    },
                    mode,
                    self.parser.screen().mouse_protocol_encoding(),
                )
            })
        };
        // Release only a previously held TUI button when a dialog takes input.
        // A lost lease still forbids even this cleanup from reaching the host.
        if (accepts_input && !overlay_pointer || release_mouse && self.backend.writable())
            && !mouse_bytes.is_empty()
        {
            self.selection = None;
            if let Err(e) = self.send(mouse_bytes) {
                self.status = e;
            }
        }
        let cell_at = |pos: egui::Pos2| {
            (
                ((pos.y - rect.min.y) / line_height)
                    .floor()
                    .clamp(0.0, f32::from(rows - 1)) as u16,
                ((pos.x - rect.min.x) / cell_width)
                    .round()
                    .clamp(0.0, f32::from(cols)) as u16,
            )
        };
        if !overlay_pointer
            && !focus_blocked
            && (!tracking || shift)
            && response.drag_started_by(egui::PointerButton::Primary)
            && let Some(pos) = ui.input(|i| i.pointer.press_origin())
        {
            let start = cell_at(pos);
            self.selection = Some(Selection { start, end: start });
        }
        if !overlay_pointer
            && !focus_blocked
            && (!tracking || shift)
            && (response.dragged_by(egui::PointerButton::Primary)
                || response.drag_stopped_by(egui::PointerButton::Primary))
            && let (Some(selection), Some(pos)) =
                (&mut self.selection, response.interact_pointer_pos())
        {
            selection.end = cell_at(pos);
        }
        if !overlay_pointer
            && !focus_blocked
            && (!tracking || shift)
            && prefs.copy_on_select
            && response.drag_stopped_by(egui::PointerButton::Primary)
            && let Some(sel) = self.selection.filter(|s| s.start != s.end)
        {
            let text = self.selection_text(sel);
            if !text.is_empty() {
                ui.ctx().copy_text(text);
            }
        }
        if !overlay_pointer && !focus_blocked && response.clicked() {
            self.selection = None;
        }
        if !overlay_pointer && !focus_blocked && (!tracking || shift) && response.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta != 0.0 {
                let release = self
                    .mouse
                    .release(self.parser.screen().mouse_protocol_encoding());
                if accepts_input
                    && !release.is_empty()
                    && let Err(e) = self.send(release)
                {
                    self.status = e;
                }
                self.scroll_by((delta / line_height).round() as i64);
            }
        }
        if !overlay_pointer && !focus_blocked && (!tracking || shift) {
            if shift || !prefs.right_click_paste {
                if is_floating {
                    response.context_menu(|ui| {
                        if self.has_selection() && ui.button(tr("Copy selection")).clicked() {
                            let _ = self.copy_selection(ui.ctx());
                            ui.close();
                        }
                        if accepts_input && ui.button(tr("Paste")).clicked() {
                            self.paste_from_clipboard();
                            ui.close();
                        }
                        if ui.button(tr("Copy visible text")).clicked() {
                            ui.ctx().copy_text(self.visible_text());
                            ui.close();
                        }
                        if ui.button(tr("Find")).clicked() {
                            self.open_find();
                            ui.close();
                        }
                        if ui.button(tr("Clear")).clicked() {
                            self.clear_screen();
                            ui.close();
                        }
                        if ui.button(tr("Reset Terminal")).clicked() {
                            self.reset_modes();
                            ui.close();
                        }
                        if ui.button(tr("Back to live output")).clicked() {
                            self.back_to_live();
                            ui.close();
                        }
                    });
                } else if response.secondary_clicked() {
                    self.shift_context_requested = ui
                        .input(|i| i.pointer.interact_pos())
                        .or_else(|| Some(rect.center()));
                }
            } else if response.secondary_clicked()
                && !self.copy_selection(ui.ctx())
                && accepts_input
            {
                self.paste_from_clipboard();
            }
        }
        // A wheel event above can enter the archive in the same frame as a key.
        // Keep the terminal focus lock, but re-evaluate actual input permission.
        let archived = self.viewing_archive();
        let accepts_input = accepts_input && !archived;
        if archived {
            self.composition.clear();
            self.composing = false;
        }
        let screen = self.parser.screen();
        let viewport = self.viewport();
        let palette = crate::theme::terminal(ui.ctx());
        let painter = ui.painter().with_clip_rect(rect);
        let default_bg = color(vt100::Color::Default, false, palette);
        painter.rect_filled(rect, 0.0, default_bg);
        // Paint every background before any glyph. Otherwise the continuation cell
        // erases the right half of a wide character painted immediately before it.
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = viewport.cell(row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let bg = if cell.inverse() {
                        color(cell.fgcolor(), true, palette)
                    } else {
                        color(cell.bgcolor(), false, palette)
                    };
                    if bg != default_bg {
                        let pos = rect.min
                            + egui::vec2(col as f32 * cell_width, row as f32 * line_height);
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                pos,
                                egui::vec2(
                                    cell_width * if cell.is_wide() { 2.0 } else { 1.0 },
                                    line_height,
                                ),
                            ),
                            0.0,
                            bg,
                        );
                    }
                }
            }
        }
        if let Some(selection) = self.selection {
            let (start, end) = selection.ordered();
            for row in start.0..=end.0 {
                let left = if row == start.0 { start.1 } else { 0 };
                let right = if row == end.0 { end.1 } else { cols };
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        rect.min
                            + egui::vec2(
                                f32::from(left) * cell_width,
                                f32::from(row) * line_height,
                            ),
                        rect.min
                            + egui::vec2(
                                f32::from(right) * cell_width,
                                f32::from(row + 1) * line_height,
                            ),
                    ),
                    0.0,
                    palette.selection_background,
                );
            }
        }
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = viewport.cell(row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let text = cell.contents();
                    let pos =
                        rect.min + egui::vec2(col as f32 * cell_width, row as f32 * line_height);
                    let fg = if cell.inverse() {
                        color(cell.bgcolor(), false, palette)
                    } else {
                        color(cell.fgcolor(), true, palette)
                    };
                    if !text.is_empty() && text != " " {
                        painter.text(pos, egui::Align2::LEFT_TOP, text, font.clone(), fg);
                    }
                    if cell.underline() {
                        let start = pos + egui::vec2(0.0, line_height - 2.0);
                        painter.line_segment(
                            [
                                start,
                                start
                                    + egui::vec2(
                                        cell_width * if cell.is_wide() { 2.0 } else { 1.0 },
                                        0.0,
                                    ),
                            ],
                            (1.0, fg),
                        );
                    }
                }
            }
        }
        if response.has_focus() && ui.is_enabled() {
            let (row, col) = screen.cursor_position();
            if accepts_input && !screen.hide_cursor() && !self.viewing_scrollback() {
                #[cfg(feature = "visual-test")]
                let blink_allowed = prefs.cursor_blink && !self.fixture;
                #[cfg(not(feature = "visual-test"))]
                let blink_allowed = prefs.cursor_blink;
                let cursor_visible = if blink_allowed {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(530));
                    let ms = (ui.input(|i| i.time.max(0.0)) * 1000.0) as u64;
                    (ms / 530).is_multiple_of(2)
                } else {
                    true
                };
                if cursor_visible {
                    let cell_origin =
                        rect.min + egui::vec2(col as f32 * cell_width, row as f32 * line_height);
                    match prefs.cursor_style {
                        crate::workspace::TerminalCursorStyle::Block => {
                            let block_rect = egui::Rect::from_min_size(
                                cell_origin,
                                egui::vec2(cell_width, line_height),
                            );
                            painter.rect_filled(
                                block_rect,
                                0.0,
                                palette.cursor.gamma_multiply(0.45_f32),
                            );
                            painter.rect_stroke(
                                block_rect,
                                0.0,
                                egui::Stroke::new(1.0_f32, palette.cursor),
                                egui::StrokeKind::Inside,
                            );
                        }
                        crate::workspace::TerminalCursorStyle::Bar => {
                            let top = cell_origin + egui::vec2(1.0_f32, 1.0_f32);
                            let bot = cell_origin + egui::vec2(1.0_f32, line_height - 1.0_f32);
                            painter.line_segment([top, bot], (2.0, palette.cursor));
                        }
                        crate::workspace::TerminalCursorStyle::Underline => {
                            let pos = rect.min
                                + egui::vec2(
                                    col as f32 * cell_width,
                                    (row as f32 + 1.0) * line_height - 2.0,
                                );
                            painter.line_segment(
                                [pos, pos + egui::vec2(cell_width, 0.0)],
                                (2.0, palette.cursor),
                            );
                        }
                    }
                }
            }
            let cursor = egui::Rect::from_min_size(
                rect.min + egui::vec2(f32::from(col) * cell_width, f32::from(row) * line_height),
                egui::vec2(cell_width, line_height),
            );
            if accepts_input {
                ui.output_mut(|o| {
                    o.ime = Some(egui::output::IMEOutput {
                        rect,
                        cursor_rect: cursor,
                    })
                });
            }
            if !self.composition.is_empty() {
                let text =
                    painter.layout_no_wrap(self.composition.clone(), font.clone(), palette.cursor);
                painter.rect_filled(
                    egui::Rect::from_min_size(cursor.min, text.size()),
                    0.0,
                    default_bg,
                );
                painter.galley(cursor.min, text, palette.cursor);
            }
            for event in ui.input(|i| i.events.clone()) {
                // Read-only panes still allow selecting and copying their visible text.
                let copying = matches!(&event, egui::Event::Copy)
                    || matches!(&event, egui::Event::Key { key: egui::Key::C, pressed: true, modifiers, .. } if modifiers.ctrl);
                if !accepts_input && !copying {
                    if archived
                        && matches!(
                            &event,
                            egui::Event::Text(_)
                                | egui::Event::Paste(_)
                                | egui::Event::Key { pressed: true, .. }
                                | egui::Event::Ime(egui::ImeEvent::Commit(_))
                        )
                    {
                        self.status = tr("History: return to live output to type.").into();
                    }
                    continue;
                }
                let mut bcast_item: Option<BroadcastInput> = None;
                let bytes = match event {
                    egui::Event::Text(text) if !self.composing => text.into_bytes(),
                    egui::Event::Ime(event) => match event {
                        egui::ImeEvent::Enabled => {
                            self.composing = true;
                            Vec::new()
                        }
                        egui::ImeEvent::Preedit(text) => {
                            self.composing = true;
                            self.composition = text.chars().take(1024).collect();
                            Vec::new()
                        }
                        egui::ImeEvent::Commit(text) => {
                            self.composing = false;
                            self.composition.clear();
                            if text.len() <= 4096 {
                                text.into_bytes()
                            } else {
                                self.status = "Composition limit: 4 KiB".into();
                                Vec::new()
                            }
                        }
                        egui::ImeEvent::Disabled => {
                            self.composing = false;
                            self.composition.clear();
                            Vec::new()
                        }
                    },
                    egui::Event::Copy => {
                        if self.copy_selection(ui.ctx()) || ui.input(|i| i.modifiers.shift) {
                            Vec::new()
                        } else {
                            vec![3]
                        }
                    }
                    egui::Event::Paste(text) => {
                        if text.len() > 65536 {
                            self.status = "Paste limit: 64 KiB".into();
                            Vec::new()
                        } else if prefs.confirm_multiline_paste
                            && text.trim_end_matches(['\r', '\n']).contains('\n')
                        {
                            self.pending_multiline_paste = Some(text);
                            Vec::new()
                        } else {
                            if !text.is_empty() {
                                bcast_item = Some(BroadcastInput::Paste(text.clone()));
                            }
                            if self.parser.screen().bracketed_paste() {
                                format!("\x1b[200~{text}\x1b[201~").into_bytes()
                            } else {
                                text.into_bytes()
                            }
                        }
                    }
                    egui::Event::Key {
                        key,
                        physical_key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if self.composing {
                            continue;
                        }
                        if modifiers.ctrl && modifiers.shift && !modifiers.alt {
                            match key {
                                egui::Key::F => {
                                    self.open_find();
                                    continue;
                                }
                                egui::Key::V => {
                                    if accepts_input {
                                        self.paste_from_clipboard();
                                    }
                                    continue;
                                }
                                egui::Key::C => {
                                    let _ = self.copy_selection(ui.ctx());
                                    continue;
                                }
                                _ => {}
                            }
                        }
                        if key == egui::Key::C
                            && modifiers.ctrl
                            && !modifiers.shift
                            && !modifiers.alt
                            && self.copy_selection(ui.ctx())
                        {
                            continue;
                        }
                        if modifiers.shift
                            && !modifiers.ctrl
                            && !modifiers.alt
                            && matches!(key, egui::Key::PageUp | egui::Key::PageDown)
                        {
                            let delta = if key == egui::Key::PageUp {
                                i64::from(rows)
                            } else {
                                -i64::from(rows)
                            };
                            self.scroll_by(delta);
                            continue;
                        }
                        let key_input = egui_to_key_input(key, physical_key, modifiers);
                        let is_alt_arrow = modifiers.alt
                            && !modifiers.ctrl
                            && !modifiers.shift
                            && matches!(
                                key,
                                egui::Key::ArrowLeft
                                    | egui::Key::ArrowRight
                                    | egui::Key::ArrowUp
                                    | egui::Key::ArrowDown
                            );
                        let extra_app_chord = modifiers.ctrl
                            && modifiers.shift
                            && !modifiers.alt
                            && matches!(key, egui::Key::T | egui::Key::J | egui::Key::Space);
                        if !(is_floating && is_alt_arrow)
                            && (extra_app_chord
                                || crate::shortcuts::is_terminal_app_shortcut(&key_input))
                        {
                            continue;
                        }
                        if key == egui::Key::Enter
                            && modifiers.shift
                            && !modifiers.ctrl
                            && !modifiers.alt
                            && let Some(agent) = self.agent_kind.as_deref()
                        {
                            if cfg!(windows) && matches!(agent, "claude" | "codex") {
                                b"\x1b\r".to_vec()
                            } else {
                                b"\n".to_vec()
                            }
                        } else {
                            let mut bytes = key_bytes(key, modifiers);
                            if bytes.len() == 3
                                && bytes[0] == 27
                                && bytes[1] == b'['
                                && matches!(bytes[2], b'A'..=b'D' | b'H' | b'F')
                            {
                                bcast_item = Some(BroadcastInput::CursorKey(bytes[2]));
                            }
                            if self.parser.screen().application_cursor()
                                && bytes.len() == 3
                                && bytes[0] == 27
                                && matches!(bytes[2], b'A'..=b'D' | b'H' | b'F')
                            {
                                bytes[1] = b'O';
                            }
                            bytes
                        }
                    }
                    _ => Vec::new(),
                };
                if archived && !bytes.is_empty() {
                    self.status = tr("History: return to live output to type.").into();
                }
                if accepts_input && !bytes.is_empty() {
                    self.back_to_live();
                }
                if accepts_input
                    && !bytes.is_empty()
                    && let Err(error) = {
                        let candidate =
                            bcast_item.unwrap_or_else(|| BroadcastInput::Raw(bytes.clone()));
                        let skip = match &candidate {
                            BroadcastInput::Raw(raw) => {
                                raw.is_empty() || crate::layout_broadcast_ui::is_terminal_reply(raw)
                            }
                            BroadcastInput::Paste(text) => {
                                text.is_empty()
                                    || crate::layout_broadcast_ui::is_terminal_reply(
                                        text.as_bytes(),
                                    )
                            }
                            BroadcastInput::CursorKey(_) => false,
                        };
                        if !skip {
                            self.broadcast_outbox.push(candidate);
                        }
                        self.send(bytes)
                    }
                {
                    self.status = error.to_string();
                }
            }
        }
        if !self.status.is_empty() {
            response.on_hover_text(&self.status);
        }
    }
}

#[derive(Clone, Copy)]
struct Selection {
    start: (u16, u16),
    end: (u16, u16),
}
impl Selection {
    fn ordered(self) -> ((u16, u16), (u16, u16)) {
        if self.start <= self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }
    #[cfg(test)]
    fn text(self, screen: &vt100::Screen) -> String {
        let (start, end) = self.ordered();
        screen.contents_between(start.0, start.1, end.0, end.1)
    }
}

fn egui_to_key_input(
    key: egui::Key,
    physical_key: Option<egui::Key>,
    modifiers: egui::Modifiers,
) -> crate::shortcuts::KeyInput {
    use egui::Key::*;
    let key_str = match key {
        A => "a",
        B => "b",
        C => "c",
        D => "d",
        E => "e",
        F => "f",
        G => "g",
        H => "h",
        I => "i",
        J => "j",
        K => "k",
        L => "l",
        M => "m",
        N => "n",
        O => "o",
        P => "p",
        Q => "q",
        R => "r",
        S => "s",
        T => "t",
        U => "u",
        V => "v",
        W => "w",
        X => "x",
        Y => "y",
        Z => "z",
        Backtick => "`",
        Equals => "=",
        Plus => "+",
        Minus => "-",
        Num0 => "0",
        Comma => ",",
        PageUp => "PageUp",
        PageDown => "PageDown",
        ArrowLeft => "ArrowLeft",
        ArrowRight => "ArrowRight",
        ArrowUp => "ArrowUp",
        ArrowDown => "ArrowDown",
        F1 => "F1",
        Space => " ",
        Enter => "Enter",
        Escape => "Escape",
        _ => "",
    };
    let code_str = match physical_key {
        Some(Backtick) => "Backquote",
        Some(Num0) => "Digit0",
        Some(Plus) => "NumpadAdd",
        Some(Minus) => "NumpadSubtract",
        _ => "",
    };
    crate::shortcuts::KeyInput {
        key: key_str.to_owned(),
        code: code_str.to_owned(),
        ctrl: modifiers.ctrl,
        shift: modifiers.shift,
        alt: modifiers.alt,
        meta: modifiers.mac_cmd,
        repeat: false,
    }
}

#[cfg(windows)]
fn read_system_clipboard() -> Option<String> {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hwnd_new_owner: *mut core::ffi::c_void) -> i32;
        fn CloseClipboard() -> i32;
        fn GetClipboardData(u_format: u32) -> *mut core::ffi::c_void;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalLock(h_mem: *mut core::ffi::c_void) -> *const u16;
        fn GlobalUnlock(h_mem: *mut core::ffi::c_void) -> i32;
        fn GlobalSize(h_mem: *mut core::ffi::c_void) -> usize;
    }
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        if OpenClipboard(core::ptr::null_mut()) == 0 {
            return None;
        }
        let handle = GetClipboardData(CF_UNICODETEXT);
        if handle.is_null() {
            let _ = CloseClipboard();
            return None;
        }
        let max_units = GlobalSize(handle) / core::mem::size_of::<u16>();
        let ptr = GlobalLock(handle);
        let result = if ptr.is_null() || max_units == 0 {
            None
        } else {
            let capped = max_units.min(65537);
            let slice = core::slice::from_raw_parts(ptr, capped);
            let len = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
            Some(String::from_utf16_lossy(&slice[..len]))
        };
        let _ = GlobalUnlock(handle);
        let _ = CloseClipboard();
        result.filter(|s| !s.is_empty())
    }
}

#[cfg(not(windows))]
fn read_system_clipboard() -> Option<String> {
    None
}

#[cfg(windows)]
pub fn trigger_voice_typing() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
        VIRTUAL_KEY,
    };
    let key_ev = |vk: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // VK_LWIN = 0x5B, 'H' = 0x48
    let inputs = [
        key_ev(0x5B, false),
        key_ev(0x48, false),
        key_ev(0x48, true),
        key_ev(0x5B, true),
    ];
    unsafe {
        let _ = SendInput(&inputs, core::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(not(windows))]
pub fn trigger_voice_typing() {}

fn key_bytes(key: egui::Key, modifiers: egui::Modifiers) -> Vec<u8> {
    use egui::Key::*;
    if modifiers.ctrl {
        let letters = [
            A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
        ];
        if let Some(index) = letters.iter().position(|k| *k == key) {
            return vec![(index + 1) as u8];
        }
        if key == Space {
            return vec![0];
        }
    }
    let modifier =
        1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.ctrl);
    let final_byte = match key {
        ArrowUp => Some('A'),
        ArrowDown => Some('B'),
        ArrowRight => Some('C'),
        ArrowLeft => Some('D'),
        Home => Some('H'),
        End => Some('F'),
        _ => None,
    };
    if let Some(end) = final_byte {
        return if modifier == 1 {
            format!("\x1b[{end}")
        } else {
            format!("\x1b[1;{modifier}{end}")
        }
        .into_bytes();
    }
    let function = match key {
        F1 => Some('P'),
        F2 => Some('Q'),
        F3 => Some('R'),
        F4 => Some('S'),
        _ => None,
    };
    if let Some(end) = function {
        return if modifier == 1 {
            format!("\x1bO{end}")
        } else {
            format!("\x1b[1;{modifier}{end}")
        }
        .into_bytes();
    }
    let tilde = match key {
        Insert => Some(2),
        Delete => Some(3),
        PageUp => Some(5),
        PageDown => Some(6),
        F5 => Some(15),
        F6 => Some(17),
        F7 => Some(18),
        F8 => Some(19),
        F9 => Some(20),
        F10 => Some(21),
        F11 => Some(23),
        F12 => Some(24),
        _ => None,
    };
    if let Some(code) = tilde {
        return if modifier == 1 {
            format!("\x1b[{code}~")
        } else {
            format!("\x1b[{code};{modifier}~")
        }
        .into_bytes();
    }
    match key {
        Enter => vec![13],
        Backspace => vec![127],
        Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Tab => vec![9],
        Escape => vec![27],
        _ => vec![],
    }
}

fn color(
    value: vt100::Color,
    foreground: bool,
    palette: &crate::theme_palettes::TerminalPalette,
) -> egui::Color32 {
    match value {
        vt100::Color::Default => {
            if foreground {
                palette.foreground
            } else {
                palette.background
            }
        }
        vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
        vt100::Color::Idx(index) => {
            if index < 16
                && let Some(ansi) = palette.ansi
            {
                return ansi.as_array()[index as usize];
            }
            const BASE: [[u8; 3]; 16] = [
                [0, 0, 0],
                [205, 49, 49],
                [13, 188, 121],
                [229, 229, 16],
                [36, 114, 200],
                [188, 63, 188],
                [17, 168, 205],
                [229, 229, 229],
                [102, 102, 102],
                [241, 76, 76],
                [35, 209, 139],
                [245, 245, 67],
                [59, 142, 234],
                [214, 112, 214],
                [41, 184, 219],
                [255, 255, 255],
            ];
            let [r, g, b] = if index < 16 {
                BASE[index as usize]
            } else if index >= 232 {
                [8 + (index - 232) * 10; 3]
            } else {
                let n = index - 16;
                let component = |v| if v == 0 { 0 } else { 55 + v * 40 };
                [component(n / 36), component((n / 6) % 6), component(n % 6)]
            };
            egui::Color32::from_rgb(r, g, b)
        }
    }
}

#[cfg(windows)]
pub fn configure_fonts(ctx: &egui::Context) {
    use std::io::Read;
    let windows = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "C:/Windows".into());
    let mut fonts = egui::FontDefinitions::default();
    for (name, file) in [
        ("windows-ui", "segoeui.ttf"),
        ("windows-emoji", "seguiemj.ttf"),
        ("windows-symbols", "seguisym.ttf"),
    ] {
        let Ok(file) = std::fs::File::open(windows.join("Fonts").join(file)) else {
            continue;
        };
        let mut bytes = Vec::new();
        if file
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() > 32 * 1024 * 1024
        {
            continue;
        }
        fonts.font_data.insert(
            name.into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        if name == "windows-ui" {
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, name.into());
            continue;
        }
        for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
            fonts.families.entry(family).or_default().push(name.into());
        }
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    #[test]
    fn observed_model_cache_is_terminal_local_and_refreshes_only_from_output() {
        let models = crate::agent_model_catalog::models_for("codex", None);
        let mut terminal = super::Terminal::spawn().unwrap();
        terminal.parser = vt100::Parser::new(5, 100, 100);
        assert!(terminal.observed_model("codex", &models).is_none());
        terminal.process_output(
            "\x1b[3;1H› Ask Codex anything\x1b[5;1HGPT-5.6 Sol xhigh fast".as_bytes(),
        );
        assert_eq!(
            terminal.observed_model("codex", &models).unwrap().model,
            "gpt-5.6-sol"
        );
        assert!(!terminal.model_dirty);
        terminal.process_output(b"\x1b[2J");
        assert_eq!(
            terminal.observed_model("codex", &models).unwrap().model,
            "gpt-5.6-sol"
        );
        terminal
            .process_output("\x1b[3;1H› Ask Codex anything\x1b[5;1HGPT-5.6 Luna medium".as_bytes());
        assert_eq!(
            terminal.observed_model("codex", &models).unwrap().model,
            "gpt-5.6-luna"
        );
        drop(terminal);
        let mut fresh = super::Terminal::spawn().unwrap();
        assert!(fresh.observed_model("codex", &models).is_none());
    }
    #[test]
    fn codex_header_prefers_current_footer_then_verified_conversation() {
        let _guard = crate::pane_model_worker::TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("profile");
        std::fs::create_dir_all(root.join("archived_sessions")).unwrap();
        let id = "12345678-1234-4234-8234-123456789abc";
        let metadata = serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":dir.path(),"timestamp":"2026-11-03T00:00:00Z"}});
        let turn = serde_json::json!({"type":"turn_context","payload":{"model":"gpt-5.6-luna","effort":"medium"}});
        std::fs::write(
            root.join("archived_sessions")
                .join(format!("rollout-{id}.jsonl")),
            format!("{metadata}\n{turn}\n"),
        )
        .unwrap();
        let mut command = CommandBuilder::new("synthetic-profile");
        command.env_clear();
        command.cwd(dir.path());
        command.env("CODEX_HOME", root);
        let mut terminal = Terminal::spawn().unwrap();
        terminal.model_profile =
            crate::pane_model_profile::Profile::capture("codex", &command, dir.path());
        terminal.parser = vt100::Parser::new(5, 100, 100);
        let models = crate::agent_model_catalog::models_for("codex", None);
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let seen = loop {
            if let Some(model) = terminal.header_model("codex", &models, Some(id), &ctx) {
                break model;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(seen.model, "gpt-5.6-luna");
        assert_eq!(seen.source, crate::pane_session_model::ModelSource::Session);
        terminal.process_output(
            "\x1b[3;1H› Ask Codex anything\x1b[5;1HGPT-5.6 Sol xhigh fast".as_bytes(),
        );
        let seen = terminal
            .header_model("codex", &models, Some(id), &ctx)
            .unwrap();
        assert_eq!(seen.model, "gpt-5.6-sol");
        assert_eq!(seen.source, crate::pane_session_model::ModelSource::Screen);
        terminal.process_output(b"\x1b[2J");
        let seen = terminal
            .header_model("codex", &models, Some(id), &ctx)
            .unwrap();
        assert_eq!(seen.model, "gpt-5.6-luna");
        assert_eq!(seen.source, crate::pane_session_model::ModelSource::Session);
        assert!(
            terminal
                .header_model("claude", &models, Some(id), &ctx)
                .is_none()
        );
    }
    #[test]
    #[cfg(windows)]
    fn claude_header_reads_the_captured_profile_without_a_footer() {
        let _guard = crate::pane_model_worker::TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("profile/projects/synthetic");
        std::fs::create_dir_all(&project).unwrap();
        let id = "12345678-1234-4234-8234-123456789abc";
        let turn = serde_json::json!({"type":"assistant", "sessionId":id, "cwd":dir.path(),
            "message":{"model":"claude-opus-4-8"}, "perTurnEffort":"high"});
        std::fs::write(project.join(format!("{id}.jsonl")), format!("{turn}\n")).unwrap();
        let mut command = CommandBuilder::new("synthetic-profile");
        command.env_clear();
        command.cwd(dir.path());
        command.env("CLAUDE_CONFIG_DIR", dir.path().join("profile"));
        let mut terminal = Terminal::spawn().unwrap();
        terminal.model_profile =
            crate::pane_model_profile::Profile::capture("claude", &command, dir.path());
        let ctx = egui::Context::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let seen = loop {
            if let Some(model) = terminal.header_model("claude", &[], Some(id), &ctx) {
                break model;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(seen.model, "claude-opus-4-8");
        assert_eq!(seen.effort.as_deref(), Some("high"));
        assert_eq!(seen.source, crate::pane_session_model::ModelSource::Session);
        assert!(
            terminal
                .header_model("openclaude", &[], Some(id), &ctx)
                .is_none()
        );
    }
    #[test]
    #[cfg(windows)]
    fn restart_stop_accepts_a_process_that_exited_with_code_259() {
        let mut command = portable_pty::CommandBuilder::new("powershell.exe");
        command.args(["-NoProfile", "-Command", "exit 259"]);
        let mut terminal = super::Terminal::spawn_command(command).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !terminal.exited() {
            terminal.poll();
            assert!(
                std::time::Instant::now() < deadline,
                "Exited process was misclassified as running"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        terminal.stop_for_restart().unwrap();
    }
    #[test]
    #[cfg(windows)]
    fn restart_stop_owns_descendants_even_after_the_shell_exits() {
        use windows::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        for parent_exits in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let child_script = dir.path().join("child.ps1");
            std::fs::write(&child_script, r#"[IO.File]::WriteAllText((Join-Path $env:SYNTHETIC_ROOT 'child.pid'), [string]$PID)
while ($true) { Start-Sleep -Milliseconds 50 }"#).unwrap();
            let parent = dir.path().join("parent.ps1");
            std::fs::write(&parent, format!(r#"$child = Start-Process powershell.exe -ArgumentList @('-NoProfile','-File', ('"' + (Join-Path $env:SYNTHETIC_ROOT 'child.ps1') + '"')) -WindowStyle Hidden -PassThru
{}
"#, if parent_exits { "exit 0" } else { "while ($true) { Start-Sleep -Milliseconds 50 }" })).unwrap();
            let mut command = portable_pty::CommandBuilder::new("powershell.exe");
            command.args(["-NoProfile", "-File"]);
            command.arg(&parent);
            command.env("SYNTHETIC_ROOT", dir.path());
            let mut terminal = super::Terminal::spawn_command(command).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            let pid = loop {
                terminal.poll();
                if let Ok(text) = std::fs::read_to_string(dir.path().join("child.pid"))
                    && let Ok(pid) = text.parse::<u32>()
                {
                    break pid;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "Child fixture did not start"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            };
            let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid).unwrap() };
            if parent_exits {
                while !terminal.exited() {
                    terminal.poll();
                    assert!(
                        std::time::Instant::now() < deadline,
                        "Parent fixture did not exit"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                assert!(!terminal.backend.process_tree_exited());
            }
            let result = terminal.stop_for_restart();
            let exited = unsafe { WaitForSingleObject(process, 1000) };
            unsafe {
                CloseHandle(process).unwrap();
            }
            result.unwrap();
            assert_eq!(exited, WAIT_OBJECT_0);
            assert!(terminal.backend.process_tree_exited());
        }
    }
    use super::*;
    #[test]
    fn theme_switches_defaults_and_ansi_without_rewriting_truecolor_or_extended_colors() {
        use crate::theme_palettes::ThemeId;
        let classic = &ThemeId::Classic.spec().term;
        let warp = &ThemeId::Warp.spec().term;
        let dracula = &ThemeId::Dracula.spec().term;
        assert_ne!(
            color(vt100::Color::Default, false, classic),
            color(vt100::Color::Default, false, warp)
        );
        for index in 0..=255 {
            assert_eq!(
                color(vt100::Color::Idx(index), true, classic),
                color(vt100::Color::Idx(index), true, warp)
            );
            if index >= 16 {
                assert_eq!(
                    color(vt100::Color::Idx(index), true, classic),
                    color(vt100::Color::Idx(index), true, dracula)
                );
            }
        }
        assert_eq!(
            color(vt100::Color::Idx(1), true, dracula),
            dracula.ansi.unwrap().red
        );
        assert_eq!(
            color(vt100::Color::Rgb(7, 11, 13), true, dracula),
            egui::Color32::from_rgb(7, 11, 13)
        );
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, true, Some(ThemeId::Nord));
        assert_eq!(crate::theme::terminal(&ctx), &ThemeId::Nord.spec().term);
        crate::theme::apply(&ctx, true, Some(ThemeId::Gruvbox));
        assert_eq!(crate::theme::terminal(&ctx), &ThemeId::Gruvbox.spec().term);
    }
    #[test]
    fn disabled_terminal_cannot_receive_modal_typing_or_mouse_clicks() {
        let mut terminal = Terminal::spawn().unwrap();
        let (sender, receiver) = mpsc::sync_channel(32);
        terminal.backend.replace_input(sender);
        terminal.process_output(b"\x1b[?1000h\x1b[?1006h");
        let ctx = egui::Context::default();
        let mut render = |enabled: bool, events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if !enabled {
                            ui.disable();
                        }
                        terminal.show(ui, 14.0);
                    });
                },
            );
            terminal.has_focus(&ctx)
        };
        render(true, vec![]);
        let pos = egui::pos2(50.0, 50.0);
        let click = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        assert!(render(true, click.clone()));
        let packets: Vec<Vec<u8>> = receiver.try_iter().collect();
        assert!(packets.iter().any(|packet| packet.starts_with(b"\x1b[<")));
        render(true, vec![egui::Event::Text("enabled".into())]);
        assert_eq!(receiver.try_recv().unwrap(), b"enabled");
        render(
            false,
            vec![
                egui::Event::Text("private modal input".into()),
                egui::Event::Paste("paste".into()),
            ],
        );
        render(false, click);
        assert!(receiver.try_recv().is_err());
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("settings-text-field")));
        assert!(!terminal.has_focus(&ctx));
    }
    #[test]
    fn broadcast_captures_user_input_skips_mouse_and_adapts_target_modes() {
        let mut source = Terminal::spawn().unwrap();
        let (src_tx, src_rx) = mpsc::sync_channel(32);
        source.backend.replace_input(src_tx);
        // Source has SGR mouse tracking, bracketed paste, and application cursor keys enabled.
        source.process_output(b"\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?1h");

        let mut plain_target = Terminal::spawn().unwrap();
        let (plain_tx, plain_rx) = mpsc::sync_channel(32);
        plain_target.backend.replace_input(plain_tx);

        let mut app_target = Terminal::spawn().unwrap();
        let (app_tx, app_rx) = mpsc::sync_channel(32);
        app_target.backend.replace_input(app_tx);
        app_target.process_output(b"\x1b[?2004h\x1b[?1h");

        let mut exited_target = Terminal::spawn().unwrap();
        let (exit_tx, exit_rx) = mpsc::sync_channel(32);
        exited_target.backend.replace_input(exit_tx);
        exited_target.exited = true;

        let ctx = egui::Context::default();
        let mut step = |events: Vec<egui::Event>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        source.show(ui, 14.0);
                    });
                },
            );
            source.take_broadcast_inputs()
        };

        // Warm up layout once so egui hit-testing knows the terminal widget rect.
        let _ = step(vec![]);
        // 1. Focus click sends mouse report ONLY to source, never to broadcast outbox.
        let pos = egui::pos2(50.0, 50.0);
        let out_click = step(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(out_click.is_empty(), "mouse reports must never broadcast");
        assert!(src_rx.try_iter().any(|p| p.starts_with(b"\x1b[<")));

        // 2. Typing, Paste, ArrowUp, and a synthetic DSR reply string:
        //    DSR reply is filtered; Paste and ArrowUp adapt per target terminal mode!
        let out_typing = step(vec![
            egui::Event::Text("echo hi".into()),
            egui::Event::Paste("line1\n".into()),
            egui::Event::Key {
                key: egui::Key::ArrowUp,
                physical_key: Some(egui::Key::ArrowUp),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::Text("\x1b[24;80R".into()),
        ]);
        assert_eq!(
            out_typing,
            vec![
                BroadcastInput::Raw(b"echo hi".to_vec()),
                BroadcastInput::Paste("line1\n".into()),
                BroadcastInput::CursorKey(b'A'),
            ]
        );

        for item in &out_typing {
            plain_target.send_broadcast(item);
            app_target.send_broadcast(item);
            exited_target.send_broadcast(item);
        }

        let plain_packets: Vec<Vec<u8>> = plain_rx.try_iter().collect();
        assert_eq!(
            plain_packets,
            vec![b"echo hi".to_vec(), b"line1\n".to_vec(), b"\x1b[A".to_vec(),]
        );

        let app_packets: Vec<Vec<u8>> = app_rx.try_iter().collect();
        assert_eq!(
            app_packets,
            vec![
                b"echo hi".to_vec(),
                b"\x1b[200~line1\n\x1b[201~".to_vec(),
                b"\x1bOA".to_vec(),
            ]
        );

        assert!(
            exit_rx.try_recv().is_err(),
            "exited terminal must not receive broadcast"
        );
    }
    #[test]
    fn interrupt_and_navigation_are_terminal_sequences() {
        assert_eq!(key_bytes(egui::Key::C, egui::Modifiers::CTRL), vec![3]);
        assert_eq!(key_bytes(egui::Key::Enter, egui::Modifiers::NONE), b"\r");
        assert_eq!(
            key_bytes(egui::Key::ArrowUp, egui::Modifiers::NONE),
            b"\x1b[A"
        );
        assert_eq!(key_bytes(egui::Key::R, egui::Modifiers::CTRL), vec![18]);
        assert_eq!(key_bytes(egui::Key::Z, egui::Modifiers::CTRL), vec![26]);
        assert_eq!(key_bytes(egui::Key::Space, egui::Modifiers::CTRL), vec![0]);
        assert_eq!(key_bytes(egui::Key::Tab, egui::Modifiers::SHIFT), b"\x1b[Z");
        assert_eq!(
            key_bytes(egui::Key::ArrowLeft, egui::Modifiers::CTRL),
            b"\x1b[1;5D"
        );
        assert_eq!(key_bytes(egui::Key::F1, egui::Modifiers::NONE), b"\x1bOP");
        assert_eq!(
            key_bytes(egui::Key::F12, egui::Modifiers::NONE),
            b"\x1b[24~"
        );
        assert_eq!(
            key_bytes(egui::Key::PageDown, egui::Modifiers::SHIFT),
            b"\x1b[6;2~"
        );
    }
    #[test]
    fn selection_preserves_unicode_and_wrapped_lines_in_both_directions() {
        let mut parser = vt100::Parser::new(5, 10, 0);
        parser.process("A🙂B\r\nQuébec\r\n1234567890abc".as_bytes());
        assert_eq!(
            Selection {
                start: (0, 0),
                end: (0, 4)
            }
            .text(parser.screen()),
            "A🙂B"
        );
        assert_eq!(
            Selection {
                start: (1, 6),
                end: (0, 0)
            }
            .text(parser.screen()),
            "A🙂B\nQuébec"
        );
        assert_eq!(
            Selection {
                start: (2, 0),
                end: (3, 3)
            }
            .text(parser.screen()),
            "1234567890abc"
        );
        assert_eq!(
            Selection {
                start: (0, 1),
                end: (0, 1)
            }
            .text(parser.screen()),
            ""
        );
    }
    #[test]
    fn configured_agent_runs_local_command_with_arguments_and_environment() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = crate::agent_preferences::Preferences {
            command: if cfg!(windows) {
                "Write-Output"
            } else {
                "printf '%s\\n'"
            }
            .into(),
            args: if cfg!(windows) {
                "'quoted value' ; Write-Output $env:RUST_AGENT_FIXTURE"
            } else {
                "'quoted value' \"$RUST_AGENT_FIXTURE\""
            }
            .into(),
            env: "RUST_AGENT_FIXTURE=ENV_FROM_PREFERENCES".into(),
            ..Default::default()
        };
        let mut command = crate::launch::command_with_preferences(
            &crate::launch::Launch::Agent("codex".into()),
            dir.path(),
            Some(&prefs),
        )
        .unwrap();
        command.env("CODEX_HOME", dir.path().join("captured-profile"));
        let mut terminal = Terminal::spawn_agent_command("codex", command).unwrap();
        let profile = terminal
            .model_profile()
            .expect("profile captured at PTY launch");
        assert_eq!(profile.root(), dir.path().join("captured-profile"));
        assert_eq!(profile.cwd(), dir.path());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut found = false;
        while std::time::Instant::now() < deadline {
            match terminal
                .backend
                .recv_output(std::time::Duration::from_millis(200))
            {
                Ok(bytes) => {
                    terminal.process_output(&bytes);
                    let text = terminal.parser.screen().contents();
                    found = text.lines().any(|line| line.trim() == "quoted value")
                        && text
                            .lines()
                            .any(|line| line.trim() == "ENV_FROM_PREFERENCES");
                    if found {
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                Err(_) => {}
            }
        }
        assert!(
            found,
            "local shell did not receive configured arguments/environment"
        );
    }

    #[test]
    fn shell_really_runs_in_a_pty() {
        let mut terminal = Terminal::spawn().expect("create PTY");
        terminal.send(b"echo TESSEL_PTY_OK\r".to_vec()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut found = false;
        while std::time::Instant::now() < deadline {
            if let Ok(bytes) = terminal
                .backend
                .recv_output(std::time::Duration::from_millis(200))
            {
                terminal.process_output(&bytes);
                // Require output on its own line, not just the echoed command.
                found = terminal
                    .parser
                    .screen()
                    .contents()
                    .lines()
                    .any(|line| line.trim() == "TESSEL_PTY_OK");
                if found {
                    break;
                }
            }
        }
        assert!(found, "shell did not execute command");
    }
}
