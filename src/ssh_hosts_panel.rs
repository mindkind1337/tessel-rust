//! Native `egui` SSH Hosts settings panel (`SettingsSsh.vue` + `RemoteHostsSettings.vue` +
//! `RemoteHostForm.vue` + `ConfirmDialog.vue` + `PanelState.vue` parity).
//!
//! Pure UI component with zero filesystem, process, network, or `~/.ssh` I/O:
//! -Ported/adapted from Tessel's `RemoteHostsSettings.vue`, `RemoteHostForm.vue`,
//!   `remoteHosts.js`, and `remoteHosts.css` (derived from Orca `v1.4.214`, MIT License,
//!   Copyright (c) 2026 Lovecast Inc. — see `THIRD_PARTY_NOTICES.md`), with vector icon
//!   geometries from `crate::icons` and `lucide-vue-next` (`v0.577.0`, ISC / Feather MIT).
//! - Renders ONLY controls backed by [`SshHostsCapabilities`] and ONLY form fields persisted
//!   by native [`SshHostSpec`] (`alias` and optional remote `folder`), with an honest OpenSSH
//!   callout (`Uses OpenSSH and your SSH config...`).
//! - Never renders unobserved connection status dots when [`SshHostRow::status`] is `None`.
//! - Consumes `Escape` when its inner Add/Edit form or Remove confirmation dialog is open
//!   (with held-key anti-repeat) so pressing `Escape` inside `settings_surface::show`'s
//!   `egui::Modal` closes ONLY the inner dialog first without dismissing Settings.
//! - Re-validates the current draft from scratch on every submission and clears inline errors
//!   when the draft changes, avoiding stale prop-watch suppression on repeated validation errors.

use std::time::Duration;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Id, Key, Modifiers, Painter, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, Vec2, WidgetInfo, WidgetType,
};

use crate::icons::{self, Icon};

/// Maximum number of saved SSH host entries (`workspace::Workspace::validate`).
pub const MAX_SSH_HOSTS: usize = 1000;

/// Maximum UTF-8 byte length of an SSH host alias or `user@host` (`workspace::validate_host`).
pub const MAX_SSH_ALIAS_BYTES: usize = 253;

/// Maximum UTF-8 byte length of an optional remote folder (`remote::clean`).
pub const MAX_REMOTE_FOLDER_BYTES: usize = 8192;

// Exact semantic palette accents from `remoteHosts.css` (`16..31`, `115..132`, `193..198`).
const STATUS_OK_GREEN: Color32 = Color32::from_rgb(0x10, 0xb9, 0x81);
const STATUS_BUSY_AMBER: Color32 = Color32::from_rgb(0xea, 0xb3, 0x08);
const STATUS_BAD_RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);
const TEXT_DANGER_SOFT: Color32 = Color32::from_rgb(0xf8, 0x71, 0x71);
const PRIMARY_BTN_DARK_TEXT: Color32 = Color32::from_rgb(0x0b, 0x15, 0x30);

/// Pure-UI representation of `workspace::Host` (`alias` + optional remote `folder`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SshHostSpec {
    pub target_id: Option<String>,
    pub alias: String,
    pub folder: String,
    /// `None` = persistent `tmux` sessions on (default); `Some(false)` = explicitly off.
    pub persistent_sessions: Option<bool>,
    /// `tessel-host` daemon for fast `Files` / `Changes` (default on).
    pub host_daemon: bool,
}

impl Default for SshHostSpec {
    fn default() -> Self {
        Self::new(String::new(), String::new())
    }
}

impl SshHostSpec {
    /// Creates an [`SshHostSpec`] from raw `alias` and `folder` strings without mutating them.
    #[must_use]
    pub fn new(alias: impl Into<String>, folder: impl Into<String>) -> Self {
        Self {
            alias: alias.into(),
            target_id: None,
            folder: folder.into(),
            persistent_sessions: None,
            host_daemon: true,
        }
    }
}

/// Live connection state of a target, rendered **only** when genuinely observed by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshHostStatus {
    Connected,
    Connecting,
    Sleeping,
    Disconnected,
    Error,
}

/// Single row supplied to [`SshHostsPanel::show`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshHostRow<'a> {
    pub host: SshHostSpec,
    /// `None` hides status dot/text completely (e.g. OpenSSH PTY mode where auth isn't tracked).
    pub status: Option<SshHostStatus>,
    pub error: Option<&'a str>,
    pub busy: bool,
}

impl<'a> SshHostRow<'a> {
    /// Convenience constructor for an idle target with no observed connection status.
    #[must_use]
    pub fn new(host: SshHostSpec) -> Self {
        Self {
            host,
            status: None,
            error: None,
            busy: false,
        }
    }
}

/// Capabilities explicitly backed by the caller.
///
/// Defaults to all `false` so unimplemented operations (`Import`, `Test`, `Disconnect`, etc.)
/// never render inert buttons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SshHostsCapabilities {
    pub can_add: bool,
    pub can_edit: bool,
    pub can_remove: bool,
    pub can_connect: bool,
    pub can_test: bool,
    pub can_disconnect: bool,
    pub can_import_config: bool,
}

impl SshHostsCapabilities {
    /// Honest capability preset for Tessel Rust's current OpenSSH + workspace host persistence.
    #[must_use]
    pub const fn native_openssh() -> Self {
        Self {
            can_add: true,
            can_edit: true,
            can_remove: true,
            can_connect: true,
            can_test: true,
            can_disconnect: false,
            can_import_config: false,
        }
    }
}

/// Visual tone for [`SshHostsNotice`], matching `.rh-notice` (`remoteHosts.css:122-132`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SshHostsNoticeTone {
    #[default]
    Neutral,
    Ok,
    Bad,
}

/// Transient or persistent feedback line above the target list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SshHostsNotice<'a> {
    pub text: &'a str,
    pub tone: SshHostsNoticeTone,
}

/// Frame input for [`SshHostsPanel::show`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshHostsInput<'a> {
    pub hosts: &'a [SshHostRow<'a>],
    pub loaded: bool,
    pub loading: bool,
    pub load_error: Option<&'a str>,
    pub notice: Option<SshHostsNotice<'a>>,
    pub french: bool,
    pub capabilities: SshHostsCapabilities,
}

/// User-initiated action emitted by [`SshHostsPanel::show`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshHostsAction {
    /// Add (`original: None`) or update (`original: Some((index, before))`) a validated host.
    Save {
        original: Option<(usize, SshHostSpec)>,
        replacement: SshHostSpec,
    },
    /// Confirmed removal of the host expected at `index`.
    Remove { index: usize, expected: SshHostSpec },
    /// Launch / connect to the host at `index`.
    Connect { index: usize, host: SshHostSpec },
    /// Explicit `~/.ssh/config` import (emitted only when `capabilities.can_import_config`).
    ImportConfig,
    /// Explicit connection test (emitted only when `capabilities.can_test`).
    Test { index: usize, host: SshHostSpec },
    /// Explicit disconnect (emitted only when `capabilities.can_disconnect`).
    Disconnect { index: usize, host: SshHostSpec },
    /// Retry loading targets after `load_error`.
    RetryLoad,
}

/// Validation failure returned by [`validate_ssh_alias`], [`validate_remote_folder`], and
/// [`validate_ssh_host_draft`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshHostValidationError {
    AliasEmpty,
    AliasStartsWithDash,
    AliasTooLong,
    AliasBadChar(char),
    FolderTooLong,
    FolderControlChar,
    FolderParentTraversal,
    DuplicateTarget,
    TooManyTargets,
}

impl SshHostValidationError {
    /// Localized user-facing message matching Tessel's SSH host form copy.
    #[must_use]
    pub fn message(&self, french: bool) -> String {
        match self {
            Self::AliasEmpty => {
                if french {
                    "Un hôte ou un alias de config SSH est requis".to_owned()
                } else {
                    "Host or SSH config alias is required".to_owned()
                }
            }
            Self::AliasStartsWithDash => {
                if french {
                    "Utilisez un alias SSH ou utilisateur@hôte (les options commençant par « - » sont refusées)."
                        .to_owned()
                } else {
                    "Use an SSH config alias or user@host (options starting with '-' are not allowed)."
                        .to_owned()
                }
            }
            Self::AliasTooLong => {
                if french {
                    format!(
                        "Cet hôte ou cet alias dépasse la limite de {MAX_SSH_ALIAS_BYTES} octets."
                    )
                } else {
                    format!("Host or alias must be at most {MAX_SSH_ALIAS_BYTES} bytes.")
                }
            }
            Self::AliasBadChar(ch) => {
                if ch.is_whitespace() {
                    if french {
                        "ssh n'accepte pas d'espace dans un hôte. Entrez seulement l'alias ou utilisateur@hôte, par exemple deploy@203.0.113.5."
                            .to_owned()
                    } else {
                        "ssh cannot take a space in a host. Enter only the alias or user@host, like deploy@203.0.113.5."
                            .to_owned()
                    }
                } else if ch.is_control() {
                    if french {
                        "Cet hôte ou cet alias contient des caractères de contrôle que ssh ne peut pas accepter."
                            .to_owned()
                    } else {
                        "This host or alias has control characters ssh cannot take.".to_owned()
                    }
                } else if french {
                    format!(
                        "ssh n'accepte pas « {ch} » dans un hôte. Caractères permis : lettres et chiffres ASCII, ., _, -, @ et :."
                    )
                } else {
                    format!(
                        "ssh cannot take « {ch} » in a host. Allowed characters: ASCII letters, digits, ., _, -, @, and :."
                    )
                }
            }
            Self::FolderTooLong => {
                if french {
                    format!(
                        "Le dossier distant dépasse la limite de {MAX_REMOTE_FOLDER_BYTES} octets."
                    )
                } else {
                    format!("Remote folder must be at most {MAX_REMOTE_FOLDER_BYTES} bytes.")
                }
            }
            Self::FolderControlChar => {
                if french {
                    "Le dossier distant ne peut pas contenir de retour à la ligne ni de caractère de contrôle."
                        .to_owned()
                } else {
                    "Remote folder cannot contain newlines or control characters.".to_owned()
                }
            }
            Self::FolderParentTraversal => {
                if french {
                    "Le dossier distant ne peut pas contenir de segment « .. ».".to_owned()
                } else {
                    "Remote folder cannot contain '..' parent-directory segments.".to_owned()
                }
            }
            Self::DuplicateTarget => {
                if french {
                    "Cette cible SSH (même hôte et même dossier) existe déjà.".to_owned()
                } else {
                    "This SSH target (same host and folder) already exists.".to_owned()
                }
            }
            Self::TooManyTargets => {
                if french {
                    "Trop de cibles SSH.".to_owned()
                } else {
                    "Too many SSH targets.".to_owned()
                }
            }
        }
    }
}

const fn is_invisible_format_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

/// Strips invisible Unicode format characters (`Cf`: zero-width spaces, BOM, bidi marks) and
/// trims leading/trailing whitespace (including NBSP `\u{00A0}`), while preserving internal
/// whitespace so accidental spaces or `-o...` option splits are caught by [`validate_ssh_alias`].
#[must_use]
pub fn clean_ssh_alias(raw: &str) -> String {
    let filtered: String = raw
        .chars()
        .filter(|&c| !is_invisible_format_char(c))
        .collect();
    filtered.trim().to_owned()
}

/// Strips invisible Unicode format characters (`Cf`) and trims leading/trailing whitespace.
#[must_use]
pub fn clean_remote_folder(raw: &str) -> String {
    let filtered: String = raw
        .chars()
        .filter(|&c| !is_invisible_format_char(c))
        .collect();
    filtered.trim().to_owned()
}

/// Validates an already-cleaned SSH alias / `user@host` with exact equivalence to
/// `workspace::validate_host`.
pub fn validate_ssh_alias(alias: &str) -> Result<(), SshHostValidationError> {
    if alias.is_empty() {
        return Err(SshHostValidationError::AliasEmpty);
    }
    if alias.starts_with('-') {
        return Err(SshHostValidationError::AliasStartsWithDash);
    }
    if alias.len() > MAX_SSH_ALIAS_BYTES {
        return Err(SshHostValidationError::AliasTooLong);
    }
    for ch in alias.chars() {
        if !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '@' | ':')) {
            return Err(SshHostValidationError::AliasBadChar(ch));
        }
    }
    Ok(())
}

/// Validates an already-cleaned optional remote folder with exact equivalence to
/// `remote::clean`, `launch::command`, and `host_actions::validate_folder`.
pub fn validate_remote_folder(folder: &str) -> Result<(), SshHostValidationError> {
    if folder.is_empty() {
        return Ok(());
    }
    if folder.len() > MAX_REMOTE_FOLDER_BYTES {
        return Err(SshHostValidationError::FolderTooLong);
    }
    if folder.chars().any(char::is_control) {
        return Err(SshHostValidationError::FolderControlChar);
    }
    if folder.split(['/', '\\']).any(|seg| seg == "..") {
        return Err(SshHostValidationError::FolderParentTraversal);
    }
    Ok(())
}

/// Cleans and validates `(alias_raw, folder_raw)` against `existing` and `editing_index`.
///
/// - Rejects `> MAX_SSH_HOSTS` (`1000`), while allowing in-place edits when `existing.len() == 1000`.
/// - Rejects exact `(alias, folder)` duplicates across other indices, while allowing the same
///   `alias` with a different `folder` and allowing saving an edited entry unchanged.
pub fn validate_ssh_host_draft(
    alias_raw: &str,
    folder_raw: &str,
    existing: &[SshHostSpec],
    editing_index: Option<usize>,
) -> Result<SshHostSpec, SshHostValidationError> {
    let alias = clean_ssh_alias(alias_raw);
    let folder = clean_remote_folder(folder_raw);
    validate_ssh_alias(&alias)?;
    validate_remote_folder(&folder)?;

    if existing.len() > MAX_SSH_HOSTS
        || (editing_index.is_none() && existing.len() >= MAX_SSH_HOSTS)
    {
        return Err(SshHostValidationError::TooManyTargets);
    }

    let is_duplicate = existing.iter().enumerate().any(|(idx, item)| {
        Some(idx) != editing_index
            && clean_ssh_alias(&item.alias) == alias
            && clean_remote_folder(&item.folder) == folder
    });
    if is_duplicate {
        return Err(SshHostValidationError::DuplicateTarget);
    }

    Ok(SshHostSpec::new(alias, folder))
}

#[derive(Clone, Debug)]
struct FormState {
    original: Option<(usize, SshHostSpec)>,
    alias: String,
    folder: String,
    persistent_sessions: bool,
    host_daemon: bool,
    advanced_open: bool,
    error: String,
    focus_alias_on_open: bool,
}

#[derive(Clone, Debug)]
struct ConfirmRemoveState {
    index: usize,
    expected: SshHostSpec,
    focus_cancel_on_open: bool,
}

/// Stateful pure-UI SSH Hosts Settings panel (`Settings > SSH Hosts`).
#[derive(Clone, Debug, Default)]
pub struct SshHostsPanel {
    form: Option<FormState>,
    pending_remove: Option<ConfirmRemoveState>,
    selected_index: usize,
    escape_held: bool,
    enter_held: bool,
    list_action_key_held: Option<Key>,
}

impl SshHostsPanel {
    /// Creates a new [`SshHostsPanel`] with closed dialogs.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears all transient UI state, drafts, and open dialogs.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Returns `true` while the Add/Edit SSH host form is open.
    #[must_use]
    pub fn is_form_open(&self) -> bool {
        self.form.is_some()
    }

    /// Returns `true` while the Remove target confirmation dialog is open.
    #[must_use]
    pub fn is_confirm_remove_open(&self) -> bool {
        self.pending_remove.is_some()
    }

    /// Opens the Add SSH host form with empty fields and closes any pending Remove dialog.
    pub fn open_add(&mut self) {
        self.pending_remove = None;
        self.form = Some(FormState {
            original: None,
            alias: String::new(),
            folder: String::new(),
            persistent_sessions: true,
            host_daemon: true,
            advanced_open: false,
            error: String::new(),
            focus_alias_on_open: true,
        });
    }

    /// Opens the Edit SSH host form pre-populated with `host` at `index`.
    ///
    /// `Advanced` auto-expands when the host has persistent sessions or `host_daemon` turned off.
    pub fn open_edit(&mut self, index: usize, host: SshHostSpec) {
        self.pending_remove = None;
        self.selected_index = index;
        let persistent = host.persistent_sessions != Some(false);
        self.form = Some(FormState {
            alias: host.alias.clone(),
            folder: host.folder.clone(),
            persistent_sessions: persistent,
            host_daemon: host.host_daemon,
            advanced_open: !persistent || !host.host_daemon,
            original: Some((index, host)),
            error: String::new(),
            focus_alias_on_open: true,
        });
    }

    /// Returns `true` while the Add/Edit form's `Advanced` section is expanded.
    #[must_use]
    pub fn is_advanced_open(&self) -> bool {
        self.form.as_ref().is_some_and(|f| f.advanced_open)
    }

    /// Closes both the Add/Edit form and the Remove confirmation dialog without emitting an action.
    pub fn cancel_dialogs(&mut self) {
        self.form = None;
        self.pending_remove = None;
    }

    /// Displays a backend/caller error inside the Add/Edit form, keeping the current draft and
    /// `original` edit target intact (or opening an Add form with `error` if none was open).
    pub fn set_form_error(&mut self, error: impl Into<String>) {
        let text = error.into();
        if let Some(form) = &mut self.form {
            form.error = text;
        } else {
            self.pending_remove = None;
            self.form = Some(FormState {
                original: None,
                alias: String::new(),
                folder: String::new(),
                persistent_sessions: true,
                host_daemon: true,
                advanced_open: false,
                error: text,
                focus_alias_on_open: true,
            });
        }
    }

    fn sync_capabilities_and_targets(&mut self, input: &SshHostsInput<'_>) {
        if let Some(form) = &mut self.form {
            let allowed = if form.original.is_some() {
                input.capabilities.can_edit
            } else {
                input.capabilities.can_add
            };
            if !allowed {
                self.form = None;
            } else if let Some((idx, expected)) = &mut form.original
                && input
                    .hosts
                    .get(*idx)
                    .is_none_or(|row| row.host != *expected)
                && let Some(new_idx) = input.hosts.iter().position(|row| row.host == *expected)
            {
                *idx = new_idx;
            }
        }

        if let Some(rem) = &mut self.pending_remove {
            if !input.capabilities.can_remove {
                self.pending_remove = None;
            } else if input
                .hosts
                .get(rem.index)
                .is_none_or(|row| row.host != rem.expected)
            {
                if let Some(new_idx) = input.hosts.iter().position(|row| row.host == rem.expected) {
                    rem.index = new_idx;
                } else if input.loaded {
                    self.pending_remove = None;
                }
            }
        }

        if input.hosts.is_empty() {
            self.selected_index = 0;
        } else if self.selected_index >= input.hosts.len() {
            self.selected_index = input.hosts.len() - 1;
        }
    }

    fn update_key_releases(&mut self, ui: &egui::Ui, interaction_blocked: bool) {
        if interaction_blocked {
            self.escape_held = false;
            self.enter_held = false;
            self.list_action_key_held = None;
            return;
        }
        ui.input(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: false,
                    ..
                } = ev
                {
                    if *key == Key::Escape {
                        self.escape_held = false;
                    }
                    if *key == Key::Enter {
                        self.enter_held = false;
                    }
                    if self.list_action_key_held == Some(*key) {
                        self.list_action_key_held = None;
                    }
                }
            }
            if !i.key_down(Key::Escape) && !key_pressed_in_events(&i.events, Key::Escape) {
                self.escape_held = false;
            }
            if !i.key_down(Key::Enter) && !key_pressed_in_events(&i.events, Key::Enter) {
                self.enter_held = false;
            }
            if let Some(held) = self.list_action_key_held
                && !i.key_down(held)
                && !key_pressed_in_events(&i.events, held)
            {
                self.list_action_key_held = None;
            }
        });
    }

    /// Consumes `Escape` when an inner dialog (`pending_remove` or `form`) is open, or while the
    /// same `Escape` key press that closed an inner dialog is still physically held down, so
    /// `settings_surface::show`'s outer `egui::Modal` never closes prematurely.
    fn handle_inner_dialog_escape(&mut self, ui: &mut egui::Ui, interaction_blocked: bool) {
        if interaction_blocked {
            return;
        }
        let dialog_open = self.pending_remove.is_some() || self.form.is_some();
        if !dialog_open && !self.escape_held {
            return;
        }

        let escape_was_held = self.escape_held;
        let mut fresh_escape = false;
        let mut still_held = escape_was_held;
        let mut saw_escape = false;

        ui.input_mut(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key: Key::Escape,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } = ev
                {
                    saw_escape = true;
                    if *pressed
                        && !*repeat
                        && modifiers.matches_logically(Modifiers::NONE)
                        && !escape_was_held
                    {
                        fresh_escape = true;
                    }
                }
            }
            if saw_escape {
                still_held = key_held_after_events(&i.events, Key::Escape);
                // Consume all Escape events on this frame so outer `egui::Modal` stays open.
                while i.consume_key(Modifiers::NONE, Key::Escape) {}
            }
        });

        if saw_escape {
            self.escape_held = still_held;
            if fresh_escape {
                if self.pending_remove.is_some() {
                    self.pending_remove = None;
                } else if self.form.is_some() {
                    self.form = None;
                }
            }
        }
    }

    fn handle_list_keyboard_nav(
        &mut self,
        ui: &mut egui::Ui,
        list_focus_id: Id,
        input: &SshHostsInput<'_>,
        interaction_blocked: bool,
    ) -> Option<SshHostsAction> {
        if interaction_blocked
            || input.hosts.is_empty()
            || self.form.is_some()
            || self.pending_remove.is_some()
        {
            return None;
        }
        if ui.ctx().memory(|m| m.focused()) != Some(list_focus_id) {
            return None;
        }

        let held_before = self.list_action_key_held;
        let mut fresh_action_key: Option<Key> = None;
        let mut still_held = false;

        let (up, down, home, end) = ui.input_mut(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key: k @ (Key::Enter | Key::Delete | Key::Backspace | Key::F2),
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = ev
                    && modifiers.matches_logically(Modifiers::NONE)
                    && held_before != Some(*k)
                {
                    fresh_action_key = Some(*k);
                    break;
                }
            }
            if let Some(k) = fresh_action_key {
                still_held = key_held_after_events(&i.events, k);
            }
            let _ = i.consume_key(Modifiers::NONE, Key::Enter);
            let _ = i.consume_key(Modifiers::NONE, Key::Delete);
            let _ = i.consume_key(Modifiers::NONE, Key::Backspace);
            let _ = i.consume_key(Modifiers::NONE, Key::F2);
            let up = i.consume_key(Modifiers::NONE, Key::ArrowUp);
            let down = i.consume_key(Modifiers::NONE, Key::ArrowDown);
            let home = i.consume_key(Modifiers::NONE, Key::Home);
            let end = i.consume_key(Modifiers::NONE, Key::End);
            (up, down, home, end)
        });

        let last = input.hosts.len().saturating_sub(1);
        if home {
            self.selected_index = 0;
            ui.ctx().memory_mut(|m| m.request_focus(list_focus_id));
        } else if end {
            self.selected_index = last;
            ui.ctx().memory_mut(|m| m.request_focus(list_focus_id));
        } else if up {
            self.selected_index = self.selected_index.saturating_sub(1);
            ui.ctx().memory_mut(|m| m.request_focus(list_focus_id));
        } else if down {
            self.selected_index = (self.selected_index + 1).min(last);
            ui.ctx().memory_mut(|m| m.request_focus(list_focus_id));
        }

        if let Some(k) = fresh_action_key {
            self.list_action_key_held = still_held.then_some(k);
            let idx = self.selected_index.min(last);
            let row = &input.hosts[idx];
            if !row.busy {
                match k {
                    Key::Enter
                        if input.capabilities.can_connect
                            && row.status != Some(SshHostStatus::Connecting) =>
                    {
                        ui.ctx().memory_mut(|m| m.request_focus(list_focus_id));
                        return Some(SshHostsAction::Connect {
                            index: idx,
                            host: row.host.clone(),
                        });
                    }
                    Key::Delete | Key::Backspace if input.capabilities.can_remove => {
                        self.pending_remove = Some(ConfirmRemoveState {
                            index: idx,
                            expected: row.host.clone(),
                            focus_cancel_on_open: true,
                        });
                    }
                    Key::F2 if input.capabilities.can_edit => {
                        self.open_edit(idx, row.host.clone());
                    }
                    _ => {}
                }
            }
        }

        None
    }

    /// Renders the SSH Hosts settings page and returns at most one user-initiated [`SshHostsAction`].
    pub fn show(&mut self, ui: &mut egui::Ui, input: SshHostsInput<'_>) -> Option<SshHostsAction> {
        let panel_id = ui.id().with("tessel-ssh-hosts-panel");
        let list_focus_id = panel_id.with("hosts-list-focus");

        let popup_open = egui::Popup::is_any_open(ui.ctx());
        let window_focused = ui.input(|i| i.focused);
        let interaction_blocked = !ui.is_enabled() || !window_focused || popup_open;

        self.sync_capabilities_and_targets(&input);
        self.update_key_releases(ui, interaction_blocked);
        self.handle_inner_dialog_escape(ui, interaction_blocked);

        let mut action =
            self.handle_list_keyboard_nav(ui, list_focus_id, &input, interaction_blocked);

        let avail_w = ui.available_width().max(120.0_f32);
        ui.spacing_mut().item_spacing.y = 0.0_f32;

        // 1. `.set-page-head` (`SettingsSsh.vue:12-15`, `style.css:3559-3570`)
        render_page_header(ui, avail_w, input.french);
        ui.add_space(16.0_f32);

        // 2. Honest OpenSSH `.set-callout.info` (`style.css:3974-4004`, `fr.json:53`)
        render_openssh_callout(ui, avail_w, input.french);
        ui.add_space(18.0_f32);

        // 3. `.rh-head` (`RemoteHostsSettings.vue:216-244`, `remoteHosts.css:46-68`)
        if let Some(head_act) =
            self.render_pane_header(ui, panel_id, avail_w, &input, interaction_blocked)
            && action.is_none()
        {
            action = Some(head_act);
        }

        // 4. `.rh-notice` feedback lines (`RemoteHostsSettings.vue:246-247`)
        let load_error_text = input.load_error.map(str::trim).filter(|s| !s.is_empty());
        let list_failed = load_error_text.is_some() && input.hosts.is_empty();
        let list_loading =
            !list_failed && (!input.loaded || input.loading) && input.hosts.is_empty();

        if let Some(notice) = input.notice {
            let trimmed = notice.text.trim();
            if !trimmed.is_empty() {
                ui.add_space(12.0_f32);
                render_notice_line(ui, avail_w, trimmed, notice.tone);
            }
        }
        if let Some(err) = load_error_text
            && !list_failed
        {
            ui.add_space(8.0_f32);
            render_notice_line(ui, avail_w, err, SshHostsNoticeTone::Bad);
        }

        // 5. Inner modal-style dialog cards (`RemoteHostForm` / `ConfirmDialog`) rendered within
        //    the settings page bounds so clicks stay inside `settings_surface::show` and narrow
        //    300 px viewports never overflow.
        if self.form.is_some() {
            ui.add_space(16.0_f32);
            if let Some(form_act) =
                self.render_form_dialog(ui, panel_id, avail_w, &input, interaction_blocked)
                && action.is_none()
            {
                action = Some(form_act);
            }
        } else if self.pending_remove.is_some() {
            ui.add_space(16.0_f32);
            if let Some(rem_act) = self.render_confirm_remove_dialog(
                ui,
                panel_id,
                avail_w,
                &input,
                interaction_blocked,
            ) && action.is_none()
            {
                action = Some(rem_act);
            }
        }

        ui.add_space(16.0_f32);

        // 6. List body (`PanelState` loading -> `PanelState` error -> `.rh-empty` -> `.rh-list`)
        if list_loading {
            render_loading_state(ui, avail_w, input.french);
        } else if let Some(err) = load_error_text.filter(|_| list_failed) {
            if let Some(retry_act) = render_error_state(
                ui,
                panel_id.with("retry-load"),
                avail_w,
                err,
                input.loading,
                input.french,
                interaction_blocked,
            ) && action.is_none()
            {
                action = Some(retry_act);
            }
        } else if input.hosts.is_empty() {
            render_empty_state(ui, avail_w, input.french);
        } else if let Some(list_act) = self.render_hosts_list(
            ui,
            panel_id,
            list_focus_id,
            avail_w,
            &input,
            interaction_blocked,
        ) && action.is_none()
        {
            action = Some(list_act);
        }

        action
    }

    fn render_pane_header(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        avail_w: f32,
        input: &SshHostsInput<'_>,
        interaction_blocked: bool,
    ) -> Option<SshHostsAction> {
        let mut emitted = None;
        let show_import = input.capabilities.can_import_config;
        let show_add = input.capabilities.can_add;

        let import_txt = import_label(input.french);
        let add_txt = add_target_label(input.french);
        let import_w = if show_import {
            measure_rh_btn_width(ui, import_txt, true)
        } else {
            0.0_f32
        };
        let add_w = if show_add {
            measure_rh_btn_width(ui, add_txt, true)
        } else {
            0.0_f32
        };
        let actions_w = match (show_import, show_add) {
            (true, true) => import_w + 6.0_f32 + add_w,
            (true, false) => import_w,
            (false, true) => add_w,
            (false, false) => 0.0_f32,
        };

        // Stack actions cleanly below heading on narrow viewports so buttons never clip.
        let stacked = actions_w > 0.0_f32 && avail_w < actions_w + 210.0_f32;
        let text_w = if stacked || actions_w == 0.0_f32 {
            avail_w
        } else {
            (avail_w - actions_w - 12.0_f32).max(100.0_f32)
        };

        let strong = ui.visuals().strong_text_color();
        let dim = ui.visuals().weak_text_color();
        let title_g = layout_wrapped_galley(
            ui,
            text_w,
            pane_title(input.french),
            FontId::proportional(13.0_f32),
            strong,
            2,
        );
        let desc_g = layout_wrapped_galley(
            ui,
            text_w,
            pane_desc(input.french),
            FontId::proportional(12.0_f32),
            dim,
            3,
        );
        let text_h = title_g.size().y + 2.0_f32 + desc_g.size().y;

        if !stacked {
            let row_h = text_h.max(if actions_w > 0.0_f32 {
                24.0_f32
            } else {
                0.0_f32
            });
            let (_, row_rect) = ui.allocate_space(Vec2::new(avail_w, row_h));
            let text_top = row_rect.center().y - text_h * 0.5_f32;
            let title_h = title_g.size().y;
            let p = ui.painter().with_clip_rect(row_rect);
            p.galley(Pos2::new(row_rect.min.x, text_top), title_g, strong);
            p.galley(
                Pos2::new(row_rect.min.x, text_top + title_h + 2.0_f32),
                desc_g,
                dim,
            );

            if actions_w > 0.0_f32 {
                let btn_y = row_rect.center().y - 12.0_f32;
                let mut cur_x = row_rect.max.x - actions_w;
                if show_import {
                    let r = Rect::from_min_size(Pos2::new(cur_x, btn_y), Vec2::new(import_w, 24.0));
                    if draw_rh_button(
                        ui,
                        panel_id.with("btn-import"),
                        r,
                        Some(LucideIcon::Upload),
                        import_txt,
                        RhButtonStyle::Outlined,
                        !interaction_blocked && !input.loading,
                        Some(import_hint(input.french)),
                    ) {
                        emitted = Some(SshHostsAction::ImportConfig);
                    }
                    cur_x += import_w + 6.0_f32;
                }
                if show_add {
                    let r = Rect::from_min_size(Pos2::new(cur_x, btn_y), Vec2::new(add_w, 24.0));
                    if draw_rh_button(
                        ui,
                        panel_id.with("btn-add"),
                        r,
                        Some(LucideIcon::Plus),
                        add_txt,
                        RhButtonStyle::Outlined,
                        !interaction_blocked,
                        None,
                    ) {
                        self.open_add();
                    }
                }
            }
        } else {
            let (_, text_rect) = ui.allocate_space(Vec2::new(avail_w, text_h));
            let title_h = title_g.size().y;
            let p = ui.painter().with_clip_rect(text_rect);
            p.galley(text_rect.min, title_g, strong);
            p.galley(
                Pos2::new(text_rect.min.x, text_rect.min.y + title_h + 2.0_f32),
                desc_g,
                dim,
            );
            ui.add_space(8.0_f32);

            let wrap_two_rows = show_import && show_add && import_w + 6.0_f32 + add_w > avail_w;
            if wrap_two_rows {
                let (_, r1) = ui.allocate_space(Vec2::new(avail_w, 24.0_f32));
                let b1 = Rect::from_min_size(r1.min, Vec2::new(import_w.min(avail_w), 24.0_f32));
                if draw_rh_button(
                    ui,
                    panel_id.with("btn-import"),
                    b1,
                    Some(LucideIcon::Upload),
                    import_txt,
                    RhButtonStyle::Outlined,
                    !interaction_blocked && !input.loading,
                    Some(import_hint(input.french)),
                ) {
                    emitted = Some(SshHostsAction::ImportConfig);
                }
                ui.add_space(6.0_f32);
                let (_, r2) = ui.allocate_space(Vec2::new(avail_w, 24.0_f32));
                let b2 = Rect::from_min_size(r2.min, Vec2::new(add_w.min(avail_w), 24.0_f32));
                if draw_rh_button(
                    ui,
                    panel_id.with("btn-add"),
                    b2,
                    Some(LucideIcon::Plus),
                    add_txt,
                    RhButtonStyle::Outlined,
                    !interaction_blocked,
                    None,
                ) {
                    self.open_add();
                }
            } else {
                let (_, row_rect) = ui.allocate_space(Vec2::new(avail_w, 24.0_f32));
                let mut cur_x = row_rect.min.x;
                if show_import {
                    let w = import_w.min(avail_w);
                    let r =
                        Rect::from_min_size(Pos2::new(cur_x, row_rect.min.y), Vec2::new(w, 24.0));
                    if draw_rh_button(
                        ui,
                        panel_id.with("btn-import"),
                        r,
                        Some(LucideIcon::Upload),
                        import_txt,
                        RhButtonStyle::Outlined,
                        !interaction_blocked && !input.loading,
                        Some(import_hint(input.french)),
                    ) {
                        emitted = Some(SshHostsAction::ImportConfig);
                    }
                    cur_x += w + 6.0_f32;
                }
                if show_add {
                    let w = add_w.min((row_rect.max.x - cur_x).max(48.0_f32));
                    let r =
                        Rect::from_min_size(Pos2::new(cur_x, row_rect.min.y), Vec2::new(w, 24.0));
                    if draw_rh_button(
                        ui,
                        panel_id.with("btn-add"),
                        r,
                        Some(LucideIcon::Plus),
                        add_txt,
                        RhButtonStyle::Outlined,
                        !interaction_blocked,
                        None,
                    ) {
                        self.open_add();
                    }
                }
            }
        }

        emitted
    }

    fn render_form_dialog(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        avail_w: f32,
        input: &SshHostsInput<'_>,
        interaction_blocked: bool,
    ) -> Option<SshHostsAction> {
        let dialog_w = avail_w.min(576.0_f32); // `min(36rem, 100%)` (`remoteHosts.css:219`)
        let pad_x = if dialog_w < 360.0_f32 { 14 } else { 20 };
        let inner_w = (dialog_w - 2.0_f32 * f32::from(pad_x)).max(80.0_f32);

        let surface = ui.visuals().window_fill();
        let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let strong = ui.visuals().strong_text_color();
        let dim = ui.visuals().weak_text_color();

        let alias_id = panel_id.with("form-alias-input");
        let folder_id = panel_id.with("form-folder-input");
        let close_id = panel_id.with("form-close-btn");
        let cancel_id = panel_id.with("form-cancel-btn");
        let submit_id = panel_id.with("form-submit-btn");

        let mut close_clicked = false;
        let mut submit_triggered = false;

        ui.allocate_ui_with_layout(
            Vec2::new(dialog_w, 0.0_f32),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(dialog_w);
                egui::Frame::NONE
                    .fill(surface)
                    .stroke(Stroke::new(1.0_f32, border))
                    .corner_radius(CornerRadius::same(12))
                    .show(ui, |ui| {
                        ui.set_width(dialog_w);
                        ui.spacing_mut().item_spacing.y = 0.0_f32;

                        let is_editing = self.form.as_ref().is_some_and(|f| f.original.is_some());

                        // Header (`remoteHosts.css:229-276`)
                        egui::Frame::NONE
                            .inner_margin(egui::Margin {
                                left: pad_x,
                                right: pad_x,
                                top: 18,
                                bottom: 14,
                            })
                            .show(ui, |ui| {
                                ui.set_width(inner_w);
                                let head_text_w = (inner_w - 34.0_f32).max(60.0_f32);
                                let title_txt = form_title(is_editing, input.french);
                                let desc_txt = form_desc(is_editing, input.french);

                                let title_g = layout_wrapped_galley(
                                    ui,
                                    head_text_w,
                                    title_txt,
                                    FontId::proportional(16.0_f32),
                                    strong,
                                    2,
                                );
                                let desc_g = layout_wrapped_galley(
                                    ui,
                                    head_text_w,
                                    desc_txt,
                                    FontId::proportional(12.5_f32),
                                    dim,
                                    3,
                                );
                                let head_h =
                                    (title_g.size().y + 4.0_f32 + desc_g.size().y).max(28.0_f32);
                                let (_, head_rect) = ui.allocate_space(Vec2::new(inner_w, head_h));
                                let title_h = title_g.size().y;
                                let p = ui.painter().with_clip_rect(head_rect);
                                p.galley(head_rect.min, title_g, strong);
                                p.galley(
                                    Pos2::new(head_rect.min.x, head_rect.min.y + title_h + 4.0_f32),
                                    desc_g,
                                    dim,
                                );

                                let close_rect = Rect::from_min_size(
                                    Pos2::new(head_rect.max.x - 28.0_f32, head_rect.min.y),
                                    Vec2::splat(28.0_f32),
                                );
                                if draw_icon_button(
                                    ui,
                                    close_id,
                                    close_rect,
                                    LucideIcon::Close,
                                    16.0_f32,
                                    false,
                                    !interaction_blocked,
                                    close_label(input.french),
                                ) {
                                    close_clicked = true;
                                }

                                if let Some((_, orig)) =
                                    self.form.as_ref().and_then(|f| f.original.as_ref())
                                {
                                    ui.add_space(8.0_f32);
                                    render_editing_chip(ui, inner_w, orig, input.french);
                                }
                            });

                        draw_horizontal_divider(ui, dialog_w, border.gamma_multiply(0.6_f32));

                        // Body (`remoteHosts.css:277-354`)
                        egui::Frame::NONE
                            .inner_margin(egui::Margin {
                                left: pad_x,
                                right: pad_x,
                                top: 16,
                                bottom: 16,
                            })
                            .show(ui, |ui| {
                                ui.set_width(inner_w);
                                let Some(form) = &mut self.form else {
                                    return;
                                };
                                let prev_alias = form.alias.clone();
                                let prev_folder = form.folder.clone();

                                // Field 1: Alias / user@host *
                                render_field_label(ui, inner_w, alias_field_label(input.french));
                                ui.add_space(6.0_f32);
                                let alias_resp = ui.add_sized(
                                    [inner_w, 32.0_f32],
                                    egui::TextEdit::singleline(&mut form.alias)
                                        .id(alias_id)
                                        .interactive(!interaction_blocked)
                                        .hint_text(alias_placeholder(input.french)),
                                );
                                alias_resp.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::TextEdit,
                                        ui.is_enabled(),
                                        alias_field_label(input.french),
                                    )
                                });
                                if form.focus_alias_on_open && !interaction_blocked {
                                    form.focus_alias_on_open = false;
                                    ui.ctx().memory_mut(|m| m.request_focus(alias_id));
                                }
                                ui.add_space(4.0_f32);
                                render_field_hint(ui, inner_w, alias_field_hint(input.french));

                                ui.add_space(12.0_f32);

                                // Field 2: Remote folder (optional)
                                render_field_label(ui, inner_w, folder_field_label(input.french));
                                ui.add_space(6.0_f32);
                                let folder_resp = ui.add_sized(
                                    [inner_w, 32.0_f32],
                                    egui::TextEdit::singleline(&mut form.folder)
                                        .id(folder_id)
                                        .interactive(!interaction_blocked)
                                        .hint_text(folder_placeholder(input.french)),
                                );
                                folder_resp.widget_info(|| {
                                    WidgetInfo::labeled(
                                        WidgetType::TextEdit,
                                        ui.is_enabled(),
                                        folder_field_label(input.french),
                                    )
                                });
                                ui.add_space(4.0_f32);
                                render_field_hint(ui, inner_w, folder_field_hint(input.french));

                                // `Advanced` (`RemoteHostForm.vue`): two compact independent
                                // switches — persistent tmux sessions and the tessel-host daemon.
                                ui.add_space(12.0_f32);
                                let adv_label = if input.french { "Avancé" } else { "Advanced" };
                                let adv_header = format!(
                                    "{} {adv_label}",
                                    if form.advanced_open { "▾" } else { "▸" }
                                );
                                if ui
                                    .add_enabled(
                                        !interaction_blocked,
                                        egui::Button::new(adv_header).frame(false),
                                    )
                                    .clicked()
                                {
                                    form.advanced_open = !form.advanced_open;
                                }
                                if form.advanced_open {
                                    ui.add_space(6.0_f32);
                                    ui.add_enabled(
                                        !interaction_blocked,
                                        egui::Checkbox::new(
                                            &mut form.persistent_sessions,
                                            advanced_tmux_label(input.french),
                                        ),
                                    );
                                    render_field_hint(
                                        ui,
                                        inner_w,
                                        advanced_tmux_hint(input.french),
                                    );
                                    ui.add_space(6.0_f32);
                                    ui.add_enabled(
                                        !interaction_blocked,
                                        egui::Checkbox::new(
                                            &mut form.host_daemon,
                                            advanced_daemon_label(input.french),
                                        ),
                                    );
                                    render_field_hint(
                                        ui,
                                        inner_w,
                                        advanced_daemon_hint(input.french),
                                    );
                                }

                                // Clear stale inline error immediately when draft changes.
                                if form.alias != prev_alias || form.folder != prev_folder {
                                    form.error.clear();
                                }

                                // Submit on fresh Enter inside either field (with held-key anti-repeat).
                                let field_focused = alias_resp.has_focus()
                                    || folder_resp.has_focus()
                                    || alias_resp.lost_focus()
                                    || folder_resp.lost_focus();
                                if !interaction_blocked && field_focused {
                                    let enter_was_held = self.enter_held;
                                    let mut fresh_enter = false;
                                    let mut still_held = enter_was_held;
                                    ui.input_mut(|i| {
                                        for ev in &i.events {
                                            if let egui::Event::Key {
                                                key: Key::Enter,
                                                pressed: true,
                                                repeat: false,
                                                modifiers,
                                                ..
                                            } = ev
                                                && modifiers.matches_logically(Modifiers::NONE)
                                                && !enter_was_held
                                            {
                                                fresh_enter = true;
                                                break;
                                            }
                                        }
                                        if fresh_enter {
                                            still_held =
                                                key_held_after_events(&i.events, Key::Enter);
                                            let _ = i.consume_key(Modifiers::NONE, Key::Enter);
                                        }
                                    });
                                    if fresh_enter {
                                        self.enter_held = still_held;
                                        submit_triggered = true;
                                    }
                                }

                                if !form.error.is_empty() {
                                    ui.add_space(12.0_f32);
                                    render_wrapped_error(ui, inner_w, &form.error);
                                }
                            });

                        draw_horizontal_divider(ui, dialog_w, border.gamma_multiply(0.6_f32));

                        // Footer (`remoteHosts.css:342-349`)
                        let foot_bg = blend_color(surface, ui.visuals().faint_bg_color, 0.35_f32);
                        egui::Frame::NONE
                            .fill(foot_bg)
                            .corner_radius(CornerRadius {
                                nw: 0,
                                ne: 0,
                                sw: 12,
                                se: 12,
                            })
                            .inner_margin(egui::Margin {
                                left: pad_x,
                                right: pad_x,
                                top: 14,
                                bottom: 14,
                            })
                            .show(ui, |ui| {
                                ui.set_width(inner_w);
                                let cancel_txt = cancel_label(input.french);
                                let save_txt = form_submit_label(is_editing, input.french);
                                let cancel_w =
                                    measure_dialog_btn_width(ui, cancel_txt).min(inner_w);
                                let save_w = measure_dialog_btn_width(ui, save_txt).min(inner_w);

                                if cancel_w + 8.0_f32 + save_w <= inner_w {
                                    let (_, row_rect) =
                                        ui.allocate_space(Vec2::new(inner_w, 28.0_f32));
                                    let save_rect = Rect::from_min_size(
                                        Pos2::new(row_rect.max.x - save_w, row_rect.min.y),
                                        Vec2::new(save_w, 28.0_f32),
                                    );
                                    let cancel_rect = Rect::from_min_size(
                                        Pos2::new(
                                            save_rect.min.x - 8.0_f32 - cancel_w,
                                            row_rect.min.y,
                                        ),
                                        Vec2::new(cancel_w, 28.0_f32),
                                    );
                                    if draw_rh_button(
                                        ui,
                                        cancel_id,
                                        cancel_rect,
                                        None,
                                        cancel_txt,
                                        RhButtonStyle::Outlined,
                                        !interaction_blocked,
                                        None,
                                    ) {
                                        close_clicked = true;
                                    }
                                    if draw_rh_button(
                                        ui,
                                        submit_id,
                                        save_rect,
                                        None,
                                        save_txt,
                                        RhButtonStyle::Primary,
                                        !interaction_blocked,
                                        None,
                                    ) {
                                        submit_triggered = true;
                                    }
                                } else {
                                    let (_, r1) = ui.allocate_space(Vec2::new(inner_w, 28.0_f32));
                                    if draw_rh_button(
                                        ui,
                                        submit_id,
                                        r1,
                                        None,
                                        save_txt,
                                        RhButtonStyle::Primary,
                                        !interaction_blocked,
                                        None,
                                    ) {
                                        submit_triggered = true;
                                    }
                                    ui.add_space(6.0_f32);
                                    let (_, r2) = ui.allocate_space(Vec2::new(inner_w, 28.0_f32));
                                    if draw_rh_button(
                                        ui,
                                        cancel_id,
                                        r2,
                                        None,
                                        cancel_txt,
                                        RhButtonStyle::Outlined,
                                        !interaction_blocked,
                                        None,
                                    ) {
                                        close_clicked = true;
                                    }
                                }
                            });
                    });
            },
        );

        if close_clicked {
            self.form = None;
            ui.ctx().request_repaint();
            return None;
        }

        if submit_triggered {
            let form = self.form.as_mut()?;
            let existing: Vec<SshHostSpec> = input.hosts.iter().map(|r| r.host.clone()).collect();
            let editing_index = form.original.as_ref().map(|(idx, _)| *idx);

            if let Some((idx, expected)) = &form.original
                && input
                    .hosts
                    .get(*idx)
                    .is_none_or(|row| row.host != *expected)
            {
                form.error = target_not_found_label(input.french).to_owned();
                ui.ctx().request_repaint();
                return None;
            }

            match validate_ssh_host_draft(&form.alias, &form.folder, &existing, editing_index) {
                Ok(mut replacement) => {
                    replacement.persistent_sessions = if form.persistent_sessions {
                        None
                    } else {
                        Some(false)
                    };
                    replacement.host_daemon = form.host_daemon;
                    // Keep `self.form` open until the caller persists the mutation and calls
                    // `cancel_dialogs()`, so backend disk/limit errors passed to `set_form_error`
                    // preserve the user's draft and remain visible inline.
                    form.error.clear();
                    let original = form.original.clone();
                    ui.ctx().request_repaint();
                    return Some(SshHostsAction::Save {
                        original,
                        replacement,
                    });
                }
                Err(err) => {
                    form.error = err.message(input.french);
                    ui.ctx().request_repaint();
                }
            }
        }

        None
    }

    fn render_confirm_remove_dialog(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        avail_w: f32,
        input: &SshHostsInput<'_>,
        interaction_blocked: bool,
    ) -> Option<SshHostsAction> {
        let rem_snapshot = self.pending_remove.clone()?;

        let card_w = avail_w.min(420.0_f32); // `.confirm-card` (`style.css:6395`)
        let pad_x = if card_w < 340.0_f32 { 14 } else { 22 };
        let inner_w = (card_w - 2.0_f32 * f32::from(pad_x)).max(80.0_f32);

        let surface = ui.visuals().window_fill();
        let border = ui.visuals().widgets.noninteractive.bg_stroke.color;
        let strong = ui.visuals().strong_text_color();
        let dim = ui.visuals().weak_text_color();

        let cancel_id = panel_id.with("confirm-remove-cancel");
        let confirm_id = panel_id.with("confirm-remove-ok");

        let mut cancelled = false;
        let mut confirmed = false;

        ui.allocate_ui_with_layout(
            Vec2::new(card_w, 0.0_f32),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(card_w);
                egui::Frame::NONE
                    .fill(surface)
                    .stroke(Stroke::new(1.0_f32, border))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(egui::Margin {
                        left: pad_x,
                        right: pad_x,
                        top: 20,
                        bottom: 18,
                    })
                    .show(ui, |ui| {
                        ui.set_width(inner_w);
                        ui.spacing_mut().item_spacing.y = 0.0_f32;

                        let title_g = layout_wrapped_galley(
                            ui,
                            inner_w,
                            remove_title_label(input.french),
                            FontId::proportional(15.0_f32),
                            strong,
                            2,
                        );
                        let desc_g = layout_wrapped_galley(
                            ui,
                            inner_w,
                            remove_desc_label(input.french),
                            FontId::proportional(12.5_f32),
                            dim,
                            4,
                        );
                        let text_h = title_g.size().y + 6.0_f32 + desc_g.size().y;
                        let (_, text_rect) = ui.allocate_space(Vec2::new(inner_w, text_h));
                        let title_h = title_g.size().y;
                        let p = ui.painter().with_clip_rect(text_rect);
                        p.galley(text_rect.min, title_g, strong);
                        p.galley(
                            Pos2::new(text_rect.min.x, text_rect.min.y + title_h + 6.0_f32),
                            desc_g,
                            dim,
                        );

                        ui.add_space(10.0_f32);

                        // `.confirm-code` target box (`style.css:6415-6428`)
                        let target_summary = if rem_snapshot.expected.folder.trim().is_empty() {
                            rem_snapshot.expected.alias.clone()
                        } else {
                            format!(
                                "{} · {}",
                                rem_snapshot.expected.alias, rem_snapshot.expected.folder
                            )
                        };
                        let code_w = (inner_w - 20.0_f32).max(40.0_f32);
                        let code_g = layout_wrapped_galley(
                            ui,
                            code_w,
                            &target_summary,
                            FontId::monospace(12.0_f32),
                            strong,
                            3,
                        );
                        let code_h = (code_g.size().y + 16.0_f32).max(30.0_f32);
                        let (_, code_rect) = ui.allocate_space(Vec2::new(inner_w, code_h));
                        let p_code = ui.painter().with_clip_rect(code_rect);
                        p_code.rect(
                            code_rect,
                            CornerRadius::same(6),
                            ui.visuals().extreme_bg_color,
                            Stroke::new(1.0_f32, border),
                            StrokeKind::Inside,
                        );
                        p_code.galley(
                            Pos2::new(code_rect.min.x + 10.0_f32, code_rect.min.y + 8.0_f32),
                            code_g,
                            strong,
                        );

                        ui.add_space(18.0_f32);

                        let cancel_txt = cancel_label(input.french);
                        let remove_txt = remove_confirm_label(input.french);
                        let cancel_w =
                            (measure_dialog_btn_width(ui, cancel_txt) + 8.0_f32).min(inner_w);
                        let remove_w =
                            (measure_dialog_btn_width(ui, remove_txt) + 8.0_f32).min(inner_w);

                        if cancel_w + 8.0_f32 + remove_w <= inner_w {
                            let (_, row_rect) = ui.allocate_space(Vec2::new(inner_w, 30.0_f32));
                            let rem_rect = Rect::from_min_size(
                                Pos2::new(row_rect.max.x - remove_w, row_rect.min.y),
                                Vec2::new(remove_w, 30.0_f32),
                            );
                            let can_rect = Rect::from_min_size(
                                Pos2::new(rem_rect.min.x - 8.0_f32 - cancel_w, row_rect.min.y),
                                Vec2::new(cancel_w, 30.0_f32),
                            );
                            if draw_rh_button(
                                ui,
                                cancel_id,
                                can_rect,
                                None,
                                cancel_txt,
                                RhButtonStyle::Outlined,
                                !interaction_blocked,
                                None,
                            ) {
                                cancelled = true;
                            }
                            if draw_rh_button(
                                ui,
                                confirm_id,
                                rem_rect,
                                None,
                                remove_txt,
                                RhButtonStyle::Danger,
                                !interaction_blocked,
                                None,
                            ) {
                                confirmed = true;
                            }
                        } else {
                            let (_, r1) = ui.allocate_space(Vec2::new(inner_w, 30.0_f32));
                            if draw_rh_button(
                                ui,
                                cancel_id,
                                r1,
                                None,
                                cancel_txt,
                                RhButtonStyle::Outlined,
                                !interaction_blocked,
                                None,
                            ) {
                                cancelled = true;
                            }
                            ui.add_space(6.0_f32);
                            let (_, r2) = ui.allocate_space(Vec2::new(inner_w, 30.0_f32));
                            if draw_rh_button(
                                ui,
                                confirm_id,
                                r2,
                                None,
                                remove_txt,
                                RhButtonStyle::Danger,
                                !interaction_blocked,
                                None,
                            ) {
                                confirmed = true;
                            }
                        }

                        if let Some(rem) = &mut self.pending_remove
                            && rem.focus_cancel_on_open
                            && !interaction_blocked
                        {
                            rem.focus_cancel_on_open = false;
                            ui.ctx().memory_mut(|m| m.request_focus(cancel_id));
                        }
                    });
            },
        );

        if cancelled {
            self.pending_remove = None;
            ui.ctx().request_repaint();
            return None;
        }
        if confirmed {
            self.pending_remove = None;
            ui.ctx().request_repaint();
            return Some(SshHostsAction::Remove {
                index: rem_snapshot.index,
                expected: rem_snapshot.expected,
            });
        }
        None
    }

    fn render_hosts_list(
        &mut self,
        ui: &mut egui::Ui,
        panel_id: Id,
        list_focus_id: Id,
        avail_w: f32,
        input: &SshHostsInput<'_>,
        interaction_blocked: bool,
    ) -> Option<SshHostsAction> {
        let mut emitted = None;

        // Register positive-area focusable container (`Sense::focusable_noninteractive`) BEFORE
        // child cards so it sits underneath card widgets in z-order while retaining focus across
        // `egui::Context::end_pass` (including inside `ScrollArea` or 0-height layout containers).
        let list_h = {
            let ah = ui.available_height();
            if ah.is_finite() {
                ah.clamp(8.0_f32, 4096.0_f32)
            } else {
                256.0_f32
            }
        };
        let list_rect = Rect::from_min_size(ui.cursor().min, Vec2::new(avail_w, list_h));
        let focus_resp = ui.interact(list_rect, list_focus_id, Sense::focusable_noninteractive());
        focus_resp.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Label, ui.is_enabled(), pane_title(input.french))
        });

        let list_has_focus =
            !interaction_blocked && ui.ctx().memory(|m| m.focused()) == Some(list_focus_id);

        for (idx, row) in input.hosts.iter().enumerate() {
            if idx > 0 {
                ui.add_space(8.0_f32);
            }
            let is_selected = list_has_focus && self.selected_index == idx;
            if let Some(card_act) = self.render_host_card(
                ui,
                panel_id.with(("host-card", idx)),
                list_focus_id,
                avail_w,
                idx,
                row,
                is_selected,
                input,
                interaction_blocked,
            ) && emitted.is_none()
            {
                emitted = Some(card_act);
            }
        }

        if !interaction_blocked {
            ui.ctx().memory_mut(|m| {
                m.set_focus_lock_filter(
                    list_focus_id,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: false,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
            });
        }

        emitted
    }

    #[allow(clippy::too_many_arguments)]
    fn render_host_card(
        &mut self,
        ui: &mut egui::Ui,
        card_id: Id,
        list_focus_id: Id,
        avail_w: f32,
        idx: usize,
        row: &SshHostRow<'_>,
        is_selected: bool,
        input: &SshHostsInput<'_>,
        interaction_blocked: bool,
    ) -> Option<SshHostsAction> {
        let caps = input.capabilities;
        let show_edit = caps.can_edit;
        let show_remove = caps.can_remove;
        let is_connected = row.status == Some(SshHostStatus::Connected);
        let is_connecting = row.status == Some(SshHostStatus::Connecting);
        let show_disconnect = is_connected && caps.can_disconnect;
        let show_connecting = is_connecting;
        let show_test = !show_disconnect && !show_connecting && caps.can_test;
        let show_connect = !show_disconnect && !show_connecting && caps.can_connect;

        let connect_txt = connect_verb_label(row.status, input.french);
        let disconnect_txt = disconnect_label(input.french);
        let connecting_txt = connecting_btn_label(input.french);
        let test_txt = test_label(input.french);

        let mut action_widths: Vec<f32> = Vec::with_capacity(4);
        if show_edit {
            action_widths.push(28.0_f32);
        }
        if show_remove {
            action_widths.push(28.0_f32);
        }
        if show_disconnect {
            action_widths.push(measure_rh_btn_width(ui, disconnect_txt, true));
        } else if show_connecting {
            action_widths.push(measure_rh_btn_width(ui, connecting_txt, true));
        } else {
            if show_test {
                action_widths.push(measure_rh_btn_width(ui, test_txt, true));
            }
            if show_connect {
                action_widths.push(measure_rh_btn_width(ui, connect_txt, true));
            }
        }
        let actions_total_w = if action_widths.is_empty() {
            0.0_f32
        } else {
            action_widths.iter().sum::<f32>() + 4.0_f32 * (action_widths.len() - 1) as f32
        };

        let pad_x = if avail_w < 340.0_f32 {
            12.0_f32
        } else {
            16.0_f32
        };
        let pad_y = 12.0_f32;
        let content_w = (avail_w - 2.0_f32 * pad_x).max(80.0_f32);
        let stack_actions = actions_total_w > 0.0_f32 && content_w < actions_total_w + 150.0_f32;

        let main_w = if stack_actions || actions_total_w == 0.0_f32 {
            (content_w - 28.0_f32).max(48.0_f32) // 16px Server icon + 12px gap
        } else {
            (content_w - 28.0_f32 - 12.0_f32 - actions_total_w).max(60.0_f32)
        };

        let strong = ui.visuals().strong_text_color();
        let dim = ui.visuals().weak_text_color();
        let folder_trimmed = row.host.folder.trim();
        let error_trimmed = row.error.map(str::trim).filter(|s| !s.is_empty());

        let status_info = row
            .status
            .map(|st| (status_dot_color(st, dim), status_label(st, input.french)));
        let status_badge_w = status_info.as_ref().map_or(0.0_f32, |(_, lbl)| {
            let g =
                layout_single_line_galley(ui, 160.0_f32, lbl, FontId::proportional(11.0_f32), dim);
            8.0_f32 + 8.0_f32 + 6.0_f32 + g.size().x
        });
        let alias_max_w = if status_badge_w > 0.0_f32 && main_w > status_badge_w + 64.0_f32 {
            main_w - status_badge_w
        } else {
            main_w
        };
        let show_inline_status = status_badge_w > 0.0_f32 && main_w > status_badge_w + 64.0_f32;

        let alias_g = layout_single_line_galley(
            ui,
            alias_max_w,
            &row.host.alias,
            FontId::proportional(13.0_f32),
            strong,
        );
        let title_row_h = alias_g.size().y.max(18.0_f32);
        let stacked_status_h = if status_info.is_some() && !show_inline_status {
            16.0_f32
        } else {
            0.0_f32
        };
        let folder_g = (!folder_trimmed.is_empty()).then(|| {
            layout_single_line_galley(
                ui,
                main_w,
                folder_trimmed,
                FontId::proportional(12.0_f32),
                dim,
            )
        });
        let error_g = error_trimmed.map(|err| {
            layout_wrapped_galley(
                ui,
                main_w,
                err,
                FontId::proportional(12.0_f32),
                TEXT_DANGER_SOFT,
                3,
            )
        });

        let mut main_h = title_row_h + stacked_status_h;
        if let Some(fg) = &folder_g {
            main_h += 2.0_f32 + fg.size().y;
        }
        if let Some(eg) = &error_g {
            main_h += 3.0_f32 + eg.size().y;
        }

        let actions_row_h = if actions_total_w > 0.0_f32 {
            28.0_f32
        } else {
            0.0_f32
        };
        let inner_h = if stack_actions {
            main_h.max(20.0_f32) + 8.0_f32 + actions_row_h
        } else {
            main_h.max(actions_row_h).max(20.0_f32)
        };
        let card_h = inner_h + 2.0_f32 * pad_y;

        let (_, card_rect) = ui.allocate_space(Vec2::new(avail_w, card_h));

        // Background click selects row & focuses keyboard navigation without stealing button hits.
        let bg_resp = ui.interact(card_rect, card_id.with("bg"), Sense::click());
        bg_resp.widget_info(|| {
            WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &row.host.alias)
        });

        let base_fill = blend_color(
            ui.visuals().panel_fill,
            ui.visuals().faint_bg_color,
            0.45_f32,
        );
        let border_col = if is_selected {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals()
                .widgets
                .noninteractive
                .bg_stroke
                .color
                .gamma_multiply(0.55_f32)
        };
        let stroke_w = if is_selected { 1.5_f32 } else { 1.0_f32 };

        let p = ui.painter().with_clip_rect(card_rect);
        p.rect(
            card_rect,
            CornerRadius::same(8),
            base_fill,
            Stroke::new(stroke_w, border_col),
            StrokeKind::Inside,
        );

        let top_block_h = if stack_actions {
            main_h.max(20.0_f32)
        } else {
            inner_h
        };
        let icon_y = card_rect.min.y + pad_y + (top_block_h - 16.0_f32) * 0.5_f32;
        let icon_rect = Rect::from_min_size(
            Pos2::new(card_rect.min.x + pad_x, icon_y),
            Vec2::splat(16.0_f32),
        );
        paint_lucide_icon(&p, icon_rect, LucideIcon::Server, dim, ui);

        let main_x = icon_rect.max.x + 12.0_f32;
        let mut cur_y = card_rect.min.y + pad_y + (top_block_h - main_h).max(0.0_f32) * 0.5_f32;
        let alias_w = alias_g.size().x;
        p.galley(Pos2::new(main_x, cur_y), alias_g, strong);

        if let Some((dot_col, status_txt)) = status_info {
            if show_inline_status {
                let dot_cx = main_x + alias_w + 8.0_f32 + 4.0_f32;
                let dot_cy = cur_y + title_row_h * 0.5_f32;
                p.circle_filled(Pos2::new(dot_cx, dot_cy), 4.0_f32, dot_col);
                let st_g = layout_single_line_galley(
                    ui,
                    (main_x + main_w - (dot_cx + 10.0_f32)).max(24.0_f32),
                    status_txt,
                    FontId::proportional(11.0_f32),
                    dim,
                );
                let st_y = dot_cy - st_g.size().y * 0.5_f32;
                p.galley(Pos2::new(dot_cx + 10.0_f32, st_y), st_g, dim);
            } else {
                let st_row_y = cur_y + title_row_h + 2.0_f32;
                let dot_cx = main_x + 4.0_f32;
                let dot_cy = st_row_y + 7.0_f32;
                p.circle_filled(Pos2::new(dot_cx, dot_cy), 4.0_f32, dot_col);
                let st_g = layout_single_line_galley(
                    ui,
                    (main_w - 14.0_f32).max(24.0_f32),
                    status_txt,
                    FontId::proportional(11.0_f32),
                    dim,
                );
                p.galley(
                    Pos2::new(dot_cx + 10.0_f32, dot_cy - st_g.size().y * 0.5_f32),
                    st_g,
                    dim,
                );
            }
        }
        cur_y += title_row_h + stacked_status_h;

        if let Some(fg) = folder_g {
            cur_y += 2.0_f32;
            let fh = fg.size().y;
            p.galley(Pos2::new(main_x, cur_y), fg, dim);
            cur_y += fh;
        }
        if let Some(eg) = error_g {
            cur_y += 3.0_f32;
            p.galley(Pos2::new(main_x, cur_y), eg, TEXT_DANGER_SOFT);
        }

        // Action buttons (`RemoteHostsSettings.vue:271-329`)
        let mut emitted = None;
        let mut any_btn_interacted = false;

        if actions_total_w > 0.0_f32 {
            let max_actions_x = card_rect.max.x - pad_x;
            let actions_y = if stack_actions {
                card_rect.max.y - pad_y - 28.0_f32
            } else {
                card_rect.center().y - 14.0_f32
            };
            let mut cur_x = if stack_actions {
                (max_actions_x - actions_total_w).max(card_rect.min.x + pad_x)
            } else {
                max_actions_x - actions_total_w
            };

            if show_edit {
                let r = Rect::from_min_size(Pos2::new(cur_x, actions_y), Vec2::splat(28.0_f32));
                if r.max.x <= max_actions_x + 0.5_f32 {
                    if draw_icon_button(
                        ui,
                        card_id.with("edit"),
                        r,
                        LucideIcon::Pencil,
                        12.0_f32,
                        false,
                        !interaction_blocked && !row.busy,
                        edit_target_label(input.french),
                    ) {
                        any_btn_interacted = true;
                        self.open_edit(idx, row.host.clone());
                        ui.ctx().request_repaint();
                    }
                    cur_x += 32.0_f32;
                }
            }

            if show_remove {
                let r = Rect::from_min_size(Pos2::new(cur_x, actions_y), Vec2::splat(28.0_f32));
                if r.max.x <= max_actions_x + 0.5_f32 {
                    let icon = if row.busy {
                        LucideIcon::Spinner
                    } else {
                        LucideIcon::Trash2
                    };
                    if draw_icon_button(
                        ui,
                        card_id.with("remove"),
                        r,
                        icon,
                        12.0_f32,
                        true,
                        !interaction_blocked && !row.busy,
                        remove_target_label(input.french),
                    ) {
                        any_btn_interacted = true;
                        self.pending_remove = Some(ConfirmRemoveState {
                            index: idx,
                            expected: row.host.clone(),
                            focus_cancel_on_open: true,
                        });
                        ui.ctx().request_repaint();
                    }
                    cur_x += 32.0_f32;
                }
            }

            let btn_y = actions_y + 2.0_f32;
            if show_disconnect {
                let w = measure_rh_btn_width(ui, disconnect_txt, true)
                    .min((max_actions_x - cur_x).max(40.0_f32));
                let r = Rect::from_min_size(Pos2::new(cur_x, btn_y), Vec2::new(w, 24.0_f32));
                if draw_rh_button(
                    ui,
                    card_id.with("disconnect"),
                    r,
                    Some(LucideIcon::ServerOff),
                    disconnect_txt,
                    RhButtonStyle::Ghost,
                    !interaction_blocked && !row.busy,
                    None,
                ) {
                    any_btn_interacted = true;
                    emitted = Some(SshHostsAction::Disconnect {
                        index: idx,
                        host: row.host.clone(),
                    });
                }
            } else if show_connecting {
                let w = measure_rh_btn_width(ui, connecting_txt, true)
                    .min((max_actions_x - cur_x).max(40.0_f32));
                let r = Rect::from_min_size(Pos2::new(cur_x, btn_y), Vec2::new(w, 24.0_f32));
                let _ = draw_rh_button(
                    ui,
                    card_id.with("connecting"),
                    r,
                    Some(LucideIcon::Spinner),
                    connecting_txt,
                    RhButtonStyle::Ghost,
                    false,
                    None,
                );
            } else {
                if show_test {
                    let w = measure_rh_btn_width(ui, test_txt, true)
                        .min((max_actions_x - cur_x).max(40.0_f32));
                    let r = Rect::from_min_size(Pos2::new(cur_x, btn_y), Vec2::new(w, 24.0_f32));
                    let icon = if row.busy {
                        LucideIcon::Spinner
                    } else {
                        LucideIcon::MonitorSmartphone
                    };
                    if draw_rh_button(
                        ui,
                        card_id.with("test"),
                        r,
                        Some(icon),
                        test_txt,
                        RhButtonStyle::Ghost,
                        !interaction_blocked && !row.busy,
                        None,
                    ) {
                        any_btn_interacted = true;
                        emitted = Some(SshHostsAction::Test {
                            index: idx,
                            host: row.host.clone(),
                        });
                    }
                    cur_x += w + 4.0_f32;
                }
                if show_connect && cur_x < max_actions_x - 24.0_f32 {
                    let w = measure_rh_btn_width(ui, connect_txt, true)
                        .min((max_actions_x - cur_x).max(40.0_f32));
                    let r = Rect::from_min_size(Pos2::new(cur_x, btn_y), Vec2::new(w, 24.0_f32));
                    let icon = if row.busy {
                        LucideIcon::Spinner
                    } else {
                        LucideIcon::Server
                    };
                    if draw_rh_button(
                        ui,
                        card_id.with("connect"),
                        r,
                        Some(icon),
                        connect_txt,
                        RhButtonStyle::Ghost,
                        !interaction_blocked && !row.busy,
                        None,
                    ) {
                        any_btn_interacted = true;
                        emitted = Some(SshHostsAction::Connect {
                            index: idx,
                            host: row.host.clone(),
                        });
                    }
                }
            }
        }

        if !interaction_blocked && bg_resp.clicked() && !any_btn_interacted {
            self.selected_index = idx;
            ui.ctx().memory_mut(|m| {
                m.request_focus(list_focus_id);
                m.set_focus_lock_filter(
                    list_focus_id,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: false,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
            });
            ui.ctx().request_repaint();
        }

        emitted
    }
}

// --- Pure-Rust vector icons (`lucide-vue-next` v0.577.0 + `crate::icons`) ------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LucideIcon {
    Plus,
    Close,
    Cable,
    Upload,
    Server,
    ServerOff,
    Pencil,
    Trash2,
    MonitorSmartphone,
    Spinner,
}

fn paint_lucide_icon(
    painter: &Painter,
    rect: Rect,
    icon: LucideIcon,
    color: Color32,
    ui: &egui::Ui,
) {
    match icon {
        LucideIcon::Plus => icons::paint(painter, rect, Icon::Plus, color),
        LucideIcon::Close => icons::paint(painter, rect, Icon::Close, color),
        LucideIcon::Cable => icons::paint(painter, rect, Icon::Cable, color),
        LucideIcon::Spinner => {
            paint_spinner(painter, rect, color, ui);
        }
        _ => {
            let side = rect.width().min(rect.height());
            if side <= 0.0_f32 || !side.is_finite() {
                return;
            }
            let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
            let s = side / 24.0_f32;
            let pt = |x: f32, y: f32| Pos2::new(sq.min.x + x * s, sq.min.y + y * s);
            let r_xywh = |x: f32, y: f32, w: f32, h: f32| {
                Rect::from_min_size(pt(x, y), Vec2::new(w * s, h * s))
            };
            let stroke = Stroke::new((2.0_f32 * s).max(1.1_f32), color);
            let rx2 = CornerRadius::same((2.0_f32 * s).round().clamp(1.0, 8.0) as u8);

            match icon {
                LucideIcon::Server => {
                    // `lucide-vue-next` `server.js`: two 20x8 racks + LED dots at (6,6) & (6,18).
                    painter.rect_stroke(
                        r_xywh(2.0, 2.0, 20.0, 8.0),
                        rx2,
                        stroke,
                        StrokeKind::Middle,
                    );
                    painter.rect_stroke(
                        r_xywh(2.0, 14.0, 20.0, 8.0),
                        rx2,
                        stroke,
                        StrokeKind::Middle,
                    );
                    let dot_r = (1.15_f32 * s).max(0.9_f32);
                    painter.circle_filled(pt(6.0, 6.0), dot_r, color);
                    painter.circle_filled(pt(6.0, 18.0), dot_r, color);
                }
                LucideIcon::ServerOff => {
                    // `lucide-vue-next` `server-off.js`: split server racks + diagonal slash (2,2)->(22,22).
                    painter.line_segment([pt(7.0, 2.0), pt(20.0, 2.0)], stroke);
                    painter.line_segment([pt(22.0, 4.0), pt(22.0, 8.0)], stroke);
                    painter.line_segment([pt(20.0, 10.0), pt(15.0, 10.0)], stroke);
                    painter.line_segment([pt(2.0, 4.5), pt(2.0, 8.0)], stroke);
                    painter.line_segment([pt(4.0, 10.0), pt(10.0, 10.0)], stroke);
                    painter.rect_stroke(
                        r_xywh(2.0, 14.0, 18.0, 8.0),
                        rx2,
                        stroke,
                        StrokeKind::Middle,
                    );
                    painter.circle_filled(pt(6.0, 18.0), (1.15_f32 * s).max(0.9_f32), color);
                    painter.line_segment([pt(2.0, 2.0), pt(22.0, 22.0)], stroke);
                }
                LucideIcon::Upload => {
                    // `lucide-vue-next` `upload.js`: `M12 3v12`, `m17 8-5-5-5 5`, tray `M21 15v4...H5...v-4`.
                    painter.line_segment([pt(12.0, 3.0), pt(12.0, 15.0)], stroke);
                    painter.add(Shape::line(
                        vec![pt(7.0, 8.0), pt(12.0, 3.0), pt(17.0, 8.0)],
                        stroke,
                    ));
                    painter.add(Shape::line(
                        vec![
                            pt(3.0, 15.0),
                            pt(3.0, 19.5),
                            pt(4.5, 21.0),
                            pt(19.5, 21.0),
                            pt(21.0, 19.5),
                            pt(21.0, 15.0),
                        ],
                        stroke,
                    ));
                }
                LucideIcon::Pencil => {
                    // `lucide-vue-next` `pencil.js`: nib + barrel + collar `m15 5 4 4`.
                    painter.add(Shape::closed_line(
                        vec![
                            pt(2.5, 21.5),
                            pt(3.6, 16.5),
                            pt(17.2, 2.9),
                            pt(21.1, 6.8),
                            pt(7.5, 20.4),
                        ],
                        stroke,
                    ));
                    painter.line_segment([pt(15.0, 5.0), pt(19.0, 9.0)], stroke);
                }
                LucideIcon::Trash2 => {
                    // `lucide-vue-next` `trash-2.js`: top bar, handle, bin body, 2 inner ribs.
                    painter.line_segment([pt(3.0, 6.0), pt(21.0, 6.0)], stroke);
                    painter.add(Shape::line(
                        vec![pt(8.0, 6.0), pt(8.0, 3.0), pt(16.0, 3.0), pt(16.0, 6.0)],
                        stroke,
                    ));
                    painter.add(Shape::line(
                        vec![
                            pt(5.0, 6.0),
                            pt(5.0, 20.0),
                            pt(7.0, 22.0),
                            pt(17.0, 22.0),
                            pt(19.0, 20.0),
                            pt(19.0, 6.0),
                        ],
                        stroke,
                    ));
                    painter.line_segment([pt(10.0, 11.0), pt(10.0, 17.0)], stroke);
                    painter.line_segment([pt(14.0, 11.0), pt(14.0, 17.0)], stroke);
                }
                LucideIcon::MonitorSmartphone => {
                    // `lucide-vue-next` `monitor-smartphone.js`: monitor + stand + right phone rect.
                    painter.add(Shape::line(
                        vec![
                            pt(18.0, 8.0),
                            pt(18.0, 4.5),
                            pt(2.5, 4.5),
                            pt(2.5, 14.5),
                            pt(12.0, 14.5),
                        ],
                        stroke,
                    ));
                    painter.line_segment([pt(10.0, 15.0), pt(10.0, 19.0)], stroke);
                    painter.line_segment([pt(7.0, 19.0), pt(12.0, 19.0)], stroke);
                    painter.rect_stroke(
                        r_xywh(16.0, 12.0, 6.0, 10.0),
                        rx2,
                        stroke,
                        StrokeKind::Middle,
                    );
                }
                LucideIcon::Plus | LucideIcon::Close | LucideIcon::Cable | LucideIcon::Spinner => {}
            }
        }
    }
}

fn paint_spinner(painter: &Painter, rect: Rect, color: Color32, ui: &egui::Ui) {
    let side = rect.width().min(rect.height());
    if side <= 0.0_f32 || !side.is_finite() {
        return;
    }
    let radius = side * (9.0_f32 / 24.0_f32);
    let center = rect.center();
    let base_angle = (ui.input(|i| i.time) as f32) * std::f32::consts::TAU;
    let sweep = std::f32::consts::PI * 1.45_f32;
    let steps = 16_usize;
    let mut pts = Vec::with_capacity(steps + 1);
    for step in 0..=steps {
        let a = base_angle + sweep * (step as f32 / steps as f32);
        let (sa, ca) = a.sin_cos();
        pts.push(Pos2::new(center.x + radius * ca, center.y + radius * sa));
    }
    let stroke = Stroke::new((side * (2.0_f32 / 24.0_f32)).max(1.2_f32), color);
    painter.add(Shape::line(pts, stroke));
    ui.ctx().request_repaint_after(Duration::from_millis(48));
}

// --- Shared drawing & layout helpers -------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum RhButtonStyle {
    Outlined,
    Ghost,
    Primary,
    Danger,
}

fn measure_rh_btn_width(ui: &egui::Ui, label: &str, with_icon: bool) -> f32 {
    let g = layout_single_line_galley(
        ui,
        320.0_f32,
        label,
        FontId::proportional(12.0_f32),
        Color32::WHITE,
    );
    let icon_part = if with_icon {
        12.0_f32 + 6.0_f32
    } else {
        0.0_f32
    };
    (16.0_f32 + icon_part + g.size().x).ceil()
}

fn measure_dialog_btn_width(ui: &egui::Ui, label: &str) -> f32 {
    let g = layout_single_line_galley(
        ui,
        360.0_f32,
        label,
        FontId::proportional(12.5_f32),
        Color32::WHITE,
    );
    (24.0_f32 + g.size().x).ceil()
}

#[allow(clippy::too_many_arguments)]
fn draw_rh_button(
    ui: &mut egui::Ui,
    id: Id,
    rect: Rect,
    icon: Option<LucideIcon>,
    label: &str,
    style: RhButtonStyle,
    enabled: bool,
    tooltip: Option<&str>,
) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let mut resp = ui.interact(rect, id, sense);
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if let Some(tip) = tooltip {
        resp = resp.on_hover_text(tip);
    }

    let hovered = enabled && resp.hovered();
    let has_focus = enabled && resp.has_focus();
    let visuals = ui.visuals();
    let accent = visuals.selection.stroke.color;
    let strong = visuals.strong_text_color();
    let normal_text = visuals.text_color();
    let hover_bg = visuals.widgets.hovered.bg_fill;
    let border_strong = visuals.widgets.inactive.bg_stroke.color;

    let (mut fill, mut border_col, mut fg) = match style {
        RhButtonStyle::Outlined => {
            let bg = if hovered {
                hover_bg
            } else {
                Color32::TRANSPARENT
            };
            (
                bg,
                border_strong,
                if hovered { strong } else { normal_text },
            )
        }
        RhButtonStyle::Ghost => {
            let bg = if hovered {
                hover_bg
            } else {
                Color32::TRANSPARENT
            };
            (
                bg,
                Color32::TRANSPARENT,
                if hovered { strong } else { normal_text },
            )
        }
        RhButtonStyle::Primary => {
            let text_col = if color_luminance(accent) > 0.45_f32 {
                PRIMARY_BTN_DARK_TEXT
            } else {
                Color32::WHITE
            };
            let bg = if hovered {
                blend_color(accent, Color32::WHITE, 0.12_f32)
            } else {
                accent
            };
            (bg, Color32::TRANSPARENT, text_col)
        }
        RhButtonStyle::Danger => {
            let bg = blend_color(
                visuals.window_fill(),
                STATUS_BAD_RED,
                if hovered { 0.32_f32 } else { 0.22_f32 },
            );
            let b = STATUS_BAD_RED.gamma_multiply(0.75_f32);
            (bg, b, strong)
        }
    };

    if !enabled {
        fill = fill.gamma_multiply(0.5_f32);
        border_col = border_col.gamma_multiply(0.5_f32);
        fg = fg.gamma_multiply(0.5_f32);
    }

    let p = ui.painter().with_clip_rect(rect);
    p.rect(
        rect,
        CornerRadius::same(6),
        fill,
        Stroke::new(1.0_f32, border_col),
        StrokeKind::Inside,
    );
    if has_focus {
        p.rect_stroke(
            rect.shrink(1.0_f32),
            CornerRadius::same(5),
            Stroke::new(1.5_f32, accent),
            StrokeKind::Inside,
        );
    }

    let pad_x = 8.0_f32;
    let icon_reserve = if icon.is_some() { 18.0_f32 } else { 0.0_f32 };
    let text_max_w = (rect.width() - 2.0_f32 * pad_x - icon_reserve).max(12.0_f32);
    let g = layout_single_line_galley(ui, text_max_w, label, FontId::proportional(12.0_f32), fg);
    let total_inner_w = icon_reserve + g.size().x;
    let start_x = (rect.center().x - total_inner_w * 0.5_f32).max(rect.min.x + pad_x);

    if let Some(ic) = icon {
        let ic_rect = Rect::from_min_size(
            Pos2::new(start_x, rect.center().y - 6.0_f32),
            Vec2::splat(12.0_f32),
        );
        paint_lucide_icon(&p, ic_rect, ic, fg, ui);
    }
    let text_x = start_x + icon_reserve;
    let text_y = rect.center().y - g.size().y * 0.5_f32;
    p.galley(Pos2::new(text_x, text_y), g, fg);

    enabled && resp.clicked()
}

#[allow(clippy::too_many_arguments)]
fn draw_icon_button(
    ui: &mut egui::Ui,
    id: Id,
    rect: Rect,
    icon: LucideIcon,
    icon_size: f32,
    danger_hover: bool,
    enabled: bool,
    label: &str,
) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let resp = ui.interact(rect, id, sense);
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    let resp = resp.on_hover_text(label);

    let hovered = enabled && resp.hovered();
    let has_focus = enabled && resp.has_focus();
    let visuals = ui.visuals();

    let fill = if hovered {
        visuals.widgets.hovered.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let mut fg = if hovered {
        if danger_hover {
            TEXT_DANGER_SOFT
        } else {
            visuals.strong_text_color()
        }
    } else {
        visuals.weak_text_color()
    };
    if !enabled {
        fg = fg.gamma_multiply(0.5_f32);
    }

    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, CornerRadius::same(6), fill);
    if has_focus {
        p.rect_stroke(
            rect.shrink(1.0_f32),
            CornerRadius::same(5),
            Stroke::new(1.5_f32, visuals.selection.stroke.color),
            StrokeKind::Inside,
        );
    }
    let ic_rect = Rect::from_center_size(rect.center(), Vec2::splat(icon_size));
    paint_lucide_icon(&p, ic_rect, icon, fg, ui);

    enabled && resp.clicked()
}

fn render_page_header(ui: &mut egui::Ui, avail_w: f32, french: bool) {
    let strong = ui.visuals().strong_text_color();
    let dim = ui.visuals().weak_text_color();
    let title_g = layout_wrapped_galley(
        ui,
        avail_w,
        page_title(french),
        FontId::proportional(21.0_f32),
        strong,
        2,
    );
    let desc_g = layout_wrapped_galley(
        ui,
        avail_w,
        page_desc(french),
        FontId::proportional(13.0_f32),
        dim,
        3,
    );
    let title_h = title_g.size().y;
    let total_h = title_h + 5.0_f32 + desc_g.size().y;
    let (_, rect) = ui.allocate_space(Vec2::new(avail_w, total_h));
    let p = ui.painter().with_clip_rect(rect);
    p.galley(rect.min, title_g, strong);
    p.galley(
        Pos2::new(rect.min.x, rect.min.y + title_h + 5.0_f32),
        desc_g,
        dim,
    );
}

fn render_openssh_callout(ui: &mut egui::Ui, avail_w: f32, french: bool) {
    let accent = ui.visuals().selection.stroke.color;
    let text_col = ui.visuals().text_color();
    let bg = blend_color(ui.visuals().panel_fill, accent, 0.12_f32);

    let pad_x = 11.0_f32;
    let pad_y = 8.0_f32;
    let text_w = (avail_w - 2.0_f32 * pad_x - 22.0_f32).max(60.0_f32);
    let g = layout_wrapped_galley(
        ui,
        text_w,
        openssh_callout_text(french),
        FontId::proportional(12.0_f32),
        text_col,
        4,
    );
    let h = (g.size().y + 2.0_f32 * pad_y).max(32.0_f32);
    let (_, rect) = ui.allocate_space(Vec2::new(avail_w, h));
    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, CornerRadius::same(8), bg);
    let ic_rect = Rect::from_min_size(
        Pos2::new(rect.min.x + pad_x, rect.center().y - 7.0_f32),
        Vec2::splat(14.0_f32),
    );
    paint_lucide_icon(&p, ic_rect, LucideIcon::Cable, accent, ui);
    let text_y = rect.center().y - g.size().y * 0.5_f32;
    p.galley(Pos2::new(ic_rect.max.x + 8.0_f32, text_y), g, text_col);
}

fn render_notice_line(ui: &mut egui::Ui, avail_w: f32, text: &str, tone: SshHostsNoticeTone) {
    let col = match tone {
        SshHostsNoticeTone::Neutral => ui.visuals().weak_text_color(),
        SshHostsNoticeTone::Ok => STATUS_OK_GREEN,
        SshHostsNoticeTone::Bad => TEXT_DANGER_SOFT,
    };
    let g = layout_wrapped_galley(ui, avail_w, text, FontId::proportional(12.0_f32), col, 4);
    let (_, rect) = ui.allocate_space(Vec2::new(avail_w, g.size().y));
    ui.painter().with_clip_rect(rect).galley(rect.min, g, col);
}

fn render_loading_state(ui: &mut egui::Ui, avail_w: f32, french: bool) {
    // `PanelState.vue:35-44` (`:rows="2"` in `RemoteHostsSettings.vue:249`).
    let dim = ui.visuals().weak_text_color();
    let skel_col = ui.visuals().widgets.inactive.bg_fill;
    let skel_col_soft = skel_col.gamma_multiply(0.7_f32);

    let line_g = layout_single_line_galley(
        ui,
        (avail_w - 48.0_f32).max(40.0_f32),
        loading_label(french),
        FontId::proportional(12.0_f32),
        dim,
    );
    let row_h = 26.0_f32;
    let total_h = 24.0_f32 + line_g.size().y.max(16.0_f32) + 2.0_f32 * (12.0_f32 + row_h);
    let (_, rect) = ui.allocate_space(Vec2::new(avail_w, total_h));
    let p = ui.painter().with_clip_rect(rect);

    let spin_rect = Rect::from_min_size(
        Pos2::new(rect.min.x + 12.0_f32, rect.min.y + 13.0_f32),
        Vec2::splat(13.0_f32),
    );
    paint_spinner(&p, spin_rect, dim, ui);
    p.galley(
        Pos2::new(spin_rect.max.x + 8.0_f32, rect.min.y + 12.0_f32),
        line_g,
        dim,
    );

    let inner_w = (avail_w - 24.0_f32).max(40.0_f32);
    let mut y = rect.min.y + 12.0_f32 + 16.0_f32 + 12.0_f32;
    for _ in 0..2 {
        p.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.min.x + 12.0_f32, y),
                Vec2::new(inner_w * 0.8_f32, 10.0_f32),
            ),
            CornerRadius::same(3),
            skel_col,
        );
        p.rect_filled(
            Rect::from_min_size(
                Pos2::new(rect.min.x + 12.0_f32, y + 16.0_f32),
                Vec2::new(inner_w * 0.5_f32, 10.0_f32),
            ),
            CornerRadius::same(3),
            skel_col_soft,
        );
        y += row_h + 12.0_f32;
    }
}

fn render_error_state(
    ui: &mut egui::Ui,
    retry_id: Id,
    avail_w: f32,
    error_text: &str,
    busy: bool,
    french: bool,
    interaction_blocked: bool,
) -> Option<SshHostsAction> {
    // `PanelState.vue:25-31` (`kind="error"` with `Retry`).
    let pad = 12.0_f32;
    let inner_w = (avail_w - 2.0_f32 * pad).max(60.0_f32);
    let msg_g = layout_wrapped_galley(
        ui,
        inner_w,
        error_text,
        FontId::proportional(12.0_f32),
        TEXT_DANGER_SOFT,
        4,
    );
    let retry_txt = retry_label(french);
    let btn_w = measure_rh_btn_width(ui, retry_txt, busy).min(inner_w);
    let total_h = 2.0_f32 * pad + msg_g.size().y + 8.0_f32 + 24.0_f32;
    let (_, rect) = ui.allocate_space(Vec2::new(avail_w, total_h));
    let msg_h = msg_g.size().y;

    let p = ui.painter().with_clip_rect(rect);
    p.rect(
        rect,
        CornerRadius::same(8),
        blend_color(ui.visuals().panel_fill, STATUS_BAD_RED, 0.08_f32),
        Stroke::new(1.0_f32, STATUS_BAD_RED.gamma_multiply(0.35_f32)),
        StrokeKind::Inside,
    );
    p.galley(
        Pos2::new(rect.min.x + pad, rect.min.y + pad),
        msg_g,
        TEXT_DANGER_SOFT,
    );

    let btn_rect = Rect::from_min_size(
        Pos2::new(rect.min.x + pad, rect.min.y + pad + msg_h + 8.0_f32),
        Vec2::new(btn_w, 24.0_f32),
    );
    let icon = busy.then_some(LucideIcon::Spinner);
    if draw_rh_button(
        ui,
        retry_id,
        btn_rect,
        icon,
        retry_txt,
        RhButtonStyle::Outlined,
        !interaction_blocked && !busy,
        None,
    ) {
        Some(SshHostsAction::RetryLoad)
    } else {
        None
    }
}

fn render_empty_state(ui: &mut egui::Ui, avail_w: f32, french: bool) {
    // `.rh-empty` (`remoteHosts.css:133-143`): centered text inside a dashed 8px-radius box.
    let dim = ui.visuals().weak_text_color();
    let inner_w = (avail_w - 32.0_f32).max(60.0_f32);
    let g = layout_wrapped_galley(
        ui,
        inner_w,
        empty_targets_label(french),
        FontId::proportional(13.0_f32),
        dim,
        2,
    );
    let h = (g.size().y + 40.0_f32).max(58.0_f32);
    let (_, rect) = ui.allocate_space(Vec2::new(avail_w, h));

    let fill = blend_color(
        ui.visuals().panel_fill,
        ui.visuals().faint_bg_color,
        0.3_f32,
    );
    let dash_col = ui
        .visuals()
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.65_f32);

    let p = ui.painter().with_clip_rect(rect);
    p.rect_filled(rect, CornerRadius::same(8), fill);
    draw_dashed_rect(
        &p,
        rect.shrink(0.5_f32),
        8.0_f32,
        Stroke::new(1.0_f32, dash_col),
    );
    let pos = Align2::CENTER_CENTER
        .anchor_size(rect.center(), g.size())
        .min;
    p.galley(pos, g, dim);
}

fn render_editing_chip(ui: &mut egui::Ui, max_w: f32, orig: &SshHostSpec, french: bool) {
    let dim = ui.visuals().weak_text_color();
    let strong = ui.visuals().strong_text_color();
    let border = ui
        .visuals()
        .widgets
        .noninteractive
        .bg_stroke
        .color
        .gamma_multiply(0.6_f32);
    let bg = blend_color(
        ui.visuals().window_fill(),
        ui.visuals().widgets.hovered.bg_fill,
        0.25_f32,
    );
    let summary = if orig.folder.trim().is_empty() {
        format!("{} {}", editing_prefix_label(french), orig.alias)
    } else {
        format!(
            "{} {} · {}",
            editing_prefix_label(french),
            orig.alias,
            orig.folder
        )
    };
    let g = layout_single_line_galley(
        ui,
        (max_w - 20.0_f32).max(40.0_f32),
        &summary,
        FontId::proportional(11.0_f32),
        strong,
    );
    let pill_w = (g.size().x + 20.0_f32).min(max_w);
    let pill_h = (g.size().y + 8.0_f32).max(22.0_f32);
    let (_, row_rect) = ui.allocate_space(Vec2::new(max_w, pill_h));
    let pill_rect = Rect::from_min_size(row_rect.min, Vec2::new(pill_w, pill_h));
    let p = ui.painter().with_clip_rect(pill_rect);
    p.rect(
        pill_rect,
        CornerRadius::same(99),
        bg,
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    p.galley(
        Pos2::new(
            pill_rect.min.x + 10.0_f32,
            pill_rect.center().y - g.size().y * 0.5_f32,
        ),
        g,
        blend_color(dim, strong, 0.75_f32),
    );
}

fn render_field_label(ui: &mut egui::Ui, width: f32, text: &str) {
    let strong = ui.visuals().strong_text_color();
    let g = layout_wrapped_galley(ui, width, text, FontId::proportional(12.0_f32), strong, 2);
    let (_, rect) = ui.allocate_space(Vec2::new(width, g.size().y));
    ui.painter()
        .with_clip_rect(rect)
        .galley(rect.min, g, strong);
}

fn render_field_hint(ui: &mut egui::Ui, width: f32, text: &str) {
    let dim = ui.visuals().weak_text_color();
    let g = layout_wrapped_galley(ui, width, text, FontId::proportional(11.0_f32), dim, 3);
    let (_, rect) = ui.allocate_space(Vec2::new(width, g.size().y));
    ui.painter().with_clip_rect(rect).galley(rect.min, g, dim);
}

fn render_wrapped_error(ui: &mut egui::Ui, width: f32, text: &str) {
    let g = layout_wrapped_galley(
        ui,
        width,
        text,
        FontId::proportional(12.0_f32),
        TEXT_DANGER_SOFT,
        4,
    );
    let (_, rect) = ui.allocate_space(Vec2::new(width, g.size().y));
    ui.painter()
        .with_clip_rect(rect)
        .galley(rect.min, g, TEXT_DANGER_SOFT);
}

fn draw_horizontal_divider(ui: &mut egui::Ui, width: f32, color: Color32) {
    let (_, rect) = ui.allocate_space(Vec2::new(width, 1.0_f32));
    ui.painter().line_segment(
        [
            Pos2::new(rect.min.x, rect.center().y),
            Pos2::new(rect.max.x, rect.center().y),
        ],
        Stroke::new(1.0_f32, color),
    );
}

fn draw_dashed_rect(painter: &Painter, rect: Rect, inset_corner: f32, stroke: Stroke) {
    let dash = 5.0_f32;
    let gap = 4.0_f32;
    let c = inset_corner
        .min(rect.width() * 0.25_f32)
        .min(rect.height() * 0.25_f32);
    draw_dashed_segment(
        painter,
        Pos2::new(rect.min.x + c, rect.min.y),
        Pos2::new(rect.max.x - c, rect.min.y),
        dash,
        gap,
        stroke,
    );
    draw_dashed_segment(
        painter,
        Pos2::new(rect.min.x + c, rect.max.y),
        Pos2::new(rect.max.x - c, rect.max.y),
        dash,
        gap,
        stroke,
    );
    draw_dashed_segment(
        painter,
        Pos2::new(rect.min.x, rect.min.y + c),
        Pos2::new(rect.min.x, rect.max.y - c),
        dash,
        gap,
        stroke,
    );
    draw_dashed_segment(
        painter,
        Pos2::new(rect.max.x, rect.min.y + c),
        Pos2::new(rect.max.x, rect.max.y - c),
        dash,
        gap,
        stroke,
    );
}

fn draw_dashed_segment(
    painter: &Painter,
    from: Pos2,
    to: Pos2,
    dash: f32,
    gap: f32,
    stroke: Stroke,
) {
    let delta = to - from;
    let total = delta.length();
    if total <= 0.5_f32 {
        return;
    }
    let dir = delta / total;
    let mut t = 0.0_f32;
    while t < total {
        let end = (t + dash).min(total);
        painter.line_segment([from + dir * t, from + dir * end], stroke);
        t += dash + gap;
    }
}

fn layout_single_line_galley(
    ui: &egui::Ui,
    max_width: f32,
    text: &str,
    font_id: FontId,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font_id, color);
    job.wrap.max_width = max_width.max(8.0_f32);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    ui.fonts_mut(|f| f.layout_job(job))
}

fn layout_wrapped_galley(
    ui: &egui::Ui,
    max_width: f32,
    text: &str,
    font_id: FontId,
    color: Color32,
    max_rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::simple(text.to_owned(), font_id, color, max_width.max(16.0_f32));
    job.wrap.max_rows = max_rows;
    ui.fonts_mut(|f| f.layout_job(job))
}

fn blend_color(base: Color32, tint: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0_f32, 1.0_f32);
    let inv = 1.0_f32 - t;
    let r = (f32::from(base.r()) * inv + f32::from(tint.r()) * t).round() as u8;
    let g = (f32::from(base.g()) * inv + f32::from(tint.g()) * t).round() as u8;
    let b = (f32::from(base.b()) * inv + f32::from(tint.b()) * t).round() as u8;
    Color32::from_rgb(r, g, b)
}

fn color_luminance(c: Color32) -> f32 {
    (0.2126_f32 * f32::from(c.r()) + 0.7152_f32 * f32::from(c.g()) + 0.0722_f32 * f32::from(c.b()))
        / 255.0_f32
}

fn key_pressed_in_events(events: &[egui::Event], target: Key) -> bool {
    events.iter().any(|ev| {
        matches!(
            ev,
            egui::Event::Key {
                key,
                pressed: true,
                ..
            } if *key == target
        )
    })
}

fn key_held_after_events(events: &[egui::Event], target: Key) -> bool {
    let mut held = false;
    for ev in events {
        if let egui::Event::Key { key, pressed, .. } = ev
            && *key == target
        {
            held = *pressed;
        }
    }
    held
}

fn status_dot_color(status: SshHostStatus, dim: Color32) -> Color32 {
    match status {
        SshHostStatus::Connected => STATUS_OK_GREEN,
        SshHostStatus::Connecting => STATUS_BUSY_AMBER,
        SshHostStatus::Error => STATUS_BAD_RED,
        SshHostStatus::Sleeping | SshHostStatus::Disconnected => dim.gamma_multiply(0.55_f32),
    }
}

// --- Exact EN / FR strings (`SettingsSsh.vue`, `RemoteHostsSettings.vue`, `remote.json`, `fr.json`) ---

const fn page_title(french: bool) -> &'static str {
    if french { "Hôtes SSH" } else { "SSH Hosts" }
}

const fn page_desc(french: bool) -> &'static str {
    if french {
        "Utilisez des machines existantes via SSH pour les fichiers, les terminaux, Git et les espaces de travail."
    } else {
        "Use existing machines over SSH for files, terminals, Git, and workspaces."
    }
}

const fn openssh_callout_text(french: bool) -> &'static str {
    if french {
        "Utilise OpenSSH et votre configuration SSH. OpenSSH gère l’authentification et la vérification des hôtes."
    } else {
        "Uses OpenSSH and your SSH config. Authentication and host verification remain with OpenSSH."
    }
}

const fn pane_title(french: bool) -> &'static str {
    if french { "Hôtes SSH" } else { "SSH hosts" }
}

const fn pane_desc(french: bool) -> &'static str {
    if french {
        "Ajoutez une machine existante via SSH pour y exécuter des projets et des espaces de travail."
    } else {
        "Add an existing machine over SSH so projects and workspaces can run there."
    }
}

const fn import_label(french: bool) -> &'static str {
    if french { "Importer" } else { "Import" }
}

const fn import_hint(french: bool) -> &'static str {
    if french {
        "Lit ~/.ssh/config et les fichiers qu'il inclut. Les fichiers qui ressemblent à des clés (id_*, *.pem, *.key, *.pub, known_hosts) sont ignorés, et les clés IdentityFile ne sont jamais ouvertes."
    } else {
        "Reads ~/.ssh/config and the files it includes. Files that look like keys (id_*, *.pem, *.key, *.pub, known_hosts) are skipped, and the IdentityFile keys are never opened."
    }
}

const fn add_target_label(french: bool) -> &'static str {
    if french {
        "Ajouter une cible"
    } else {
        "Add target"
    }
}

const fn empty_targets_label(french: bool) -> &'static str {
    if french {
        "Aucune cible SSH configurée."
    } else {
        "No SSH targets configured."
    }
}

const fn loading_label(french: bool) -> &'static str {
    if french {
        "Chargement…"
    } else {
        "Loading…"
    }
}

const fn retry_label(french: bool) -> &'static str {
    if french { "Réessayer" } else { "Retry" }
}

const fn status_label(status: SshHostStatus, french: bool) -> &'static str {
    match (status, french) {
        (SshHostStatus::Connected, false) => "Connected",
        (SshHostStatus::Connected, true) => "Connecté",
        (SshHostStatus::Connecting, false) => "Connecting…",
        (SshHostStatus::Connecting, true) => "Connexion…",
        (SshHostStatus::Sleeping, false) => "Sleeping",
        (SshHostStatus::Sleeping, true) => "En veille",
        (SshHostStatus::Disconnected, false) => "Disconnected",
        (SshHostStatus::Disconnected, true) => "Déconnecté",
        (SshHostStatus::Error, false) => "Error",
        (SshHostStatus::Error, true) => "Erreur",
    }
}

const fn connect_verb_label(status: Option<SshHostStatus>, french: bool) -> &'static str {
    match (status, french) {
        (Some(SshHostStatus::Error), false) => "Retry",
        (Some(SshHostStatus::Error), true) => "Réessayer",
        (_, false) => "Connect",
        (_, true) => "Se connecter",
    }
}

const fn connecting_btn_label(french: bool) -> &'static str {
    if french { "Connexion" } else { "Connecting" }
}

const fn disconnect_label(french: bool) -> &'static str {
    if french { "Déconnecter" } else { "Disconnect" }
}

const fn test_label(french: bool) -> &'static str {
    if french { "Tester" } else { "Test" }
}

const fn edit_target_label(french: bool) -> &'static str {
    if french {
        "Modifier la cible"
    } else {
        "Edit target"
    }
}

const fn remove_target_label(french: bool) -> &'static str {
    if french {
        "Supprimer la cible"
    } else {
        "Remove target"
    }
}

const fn form_title(editing: bool, french: bool) -> &'static str {
    match (editing, french) {
        (false, false) => "Add SSH host",
        (false, true) => "Ajouter un hôte SSH",
        (true, false) => "Edit SSH host",
        (true, true) => "Modifier l'hôte SSH",
    }
}

const fn form_desc(editing: bool, french: bool) -> &'static str {
    match (editing, french) {
        (false, false) => "Add a persistent machine you can log into over SSH.",
        (false, true) => {
            "Ajoutez une machine permanente sur laquelle vous pouvez vous connecter en SSH."
        }
        (true, false) => {
            "Update connection details for this machine. Changes apply on next connect."
        }
        (true, true) => {
            "Mettez à jour les détails de connexion de cette machine. Les modifications s'appliquent à la prochaine connexion."
        }
    }
}

const fn editing_prefix_label(french: bool) -> &'static str {
    if french { "Modification :" } else { "Editing" }
}

const fn alias_field_label(french: bool) -> &'static str {
    if french {
        "Alias / utilisateur@hôte *"
    } else {
        "Alias / user@host *"
    }
}

const fn alias_placeholder(french: bool) -> &'static str {
    if french {
        "serveur-prod, deploy@203.0.113.5"
    } else {
        "prod-box, deploy@203.0.113.5"
    }
}

const fn alias_field_hint(french: bool) -> &'static str {
    if french {
        "Alias Host de ~/.ssh/config ou destination utilisateur@hôte."
    } else {
        "Host alias from ~/.ssh/config or user@host destination."
    }
}

const fn folder_field_label(french: bool) -> &'static str {
    if french {
        "Dossier distant (facultatif)"
    } else {
        "Remote folder (optional)"
    }
}

const fn folder_placeholder(french: bool) -> &'static str {
    if french {
        "/srv/app, ~/projet"
    } else {
        "/srv/app, ~/project"
    }
}

const fn folder_field_hint(french: bool) -> &'static str {
    if french {
        "Dossier de départ facultatif sur la machine distante."
    } else {
        "Optional starting folder on the remote machine."
    }
}

const fn cancel_label(french: bool) -> &'static str {
    if french { "Annuler" } else { "Cancel" }
}

const fn close_label(french: bool) -> &'static str {
    if french { "Fermer" } else { "Close" }
}

const fn form_submit_label(editing: bool, french: bool) -> &'static str {
    match (editing, french) {
        (false, false) => "Add Target",
        (false, true) => "Ajouter une cible",
        (true, false) => "Save Changes",
        (true, true) => "Enregistrer les modifications",
    }
}

const fn target_not_found_label(french: bool) -> &'static str {
    if french {
        "Cette cible SSH n'existe plus."
    } else {
        "This SSH target no longer exists."
    }
}

const fn remove_title_label(french: bool) -> &'static str {
    if french {
        "Supprimer la cible SSH"
    } else {
        "Remove SSH target"
    }
}

const fn remove_desc_label(french: bool) -> &'static str {
    if french {
        "La cible sera supprimée et ses terminaux et sa vue Fichiers seront fermés. Les modifications non enregistrées dans cette vue seront perdues."
    } else {
        "This will remove the target and close its terminals and Files view. Unsaved changes in that view will be lost."
    }
}

const fn remove_confirm_label(french: bool) -> &'static str {
    if french { "Supprimer" } else { "Remove" }
}

/// `Advanced` switch label: persistent SSH terminals through `tmux`.
#[must_use]
pub const fn advanced_tmux_label(french: bool) -> &'static str {
    if french {
        "Terminaux persistants (tmux)"
    } else {
        "Persistent terminals (tmux)"
    }
}

#[must_use]
pub const fn advanced_tmux_hint(french: bool) -> &'static str {
    if french {
        "Les commandes continuent si SSH se coupe ; Tessel se rattache à la reconnexion."
    } else {
        "Commands keep running if SSH drops; Tessel reattaches on reconnect."
    }
}

/// `Advanced` switch label: opt-in `tessel-host` daemon for fast Files/Changes.
#[must_use]
pub const fn advanced_daemon_label(french: bool) -> &'static str {
    if french {
        "Démon Tessel (Fichiers et Modifications rapides)"
    } else {
        "Tessel daemon (fast Files and Changes)"
    }
}

#[must_use]
pub const fn advanced_daemon_hint(french: bool) -> &'static str {
    if french {
        "Utilise tessel-host dans ~/.tessel-server sur cet hôte (activé par défaut). Désactivé : SFTP et git, comme avant."
    } else {
        "Uses tessel-host in ~/.tessel-server on this host (on by default). Off: SFTP and git, as before."
    }
}
