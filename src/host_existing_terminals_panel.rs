//! Pure `egui` modal picker for existing remote `tessel-host` terminals (`terminal.list`).
//!
//! # Architectural & Security Boundaries
//! - **Zero I/O & Zero Secrets:** Never calls `tessel-host-client`, never touches `LeaseStore`,
//!   and never receives or stores a `LeaseKey` or creation `dedup_key`. Local lease availability is
//!   passed as a pure tri-state [`SavedLease`] (`Yes`, `No`, `Unknown`) and pending creations as
//!   read-only [`PendingCreationEntry`] metadata.
//! - **Exact Protocol Identity:** `epoch` and `terminal_id` carried inside emitted
//!   [`ExistingTerminalsAction`] variants are **exact raw protocol strings** (`to_owned()` without
//!   trimming, truncation, or control stripping). Sanitization and truncation via
//!   [`sanitize_host_text`] apply strictly to rendered UI labels.
//! - **No Implicit `create`, Lease Takeover, or Cleanup:** Opening, refreshing, or pressing `Enter`
//!   on an empty/error/stale list never spawns or cleans up anything. Attaching to an existing
//!   terminal emits only `ResumeSaved` (`Target::Existing { resume: true }`, gated strictly on
//!   `saved_lease == SavedLease::Yes && running`) or `AttachObserver`
//!   (`Target::Existing { resume: false }`, read-only `Writer::None`).

use crate::host_terminal_panel::{MAX_HOST_LABEL_CHARS, sanitize_host_text};
use crate::icons::{self, Icon};
use crate::theme_palettes::{ThemeId, UiPalette};
use eframe::egui::{
    self, Align, Area, Color32, CornerRadius, CursorIcon, Event, FontId, Id, Key, Layout, Margin,
    Modifiers, Order, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2,
    WidgetInfo, WidgetType,
};
use std::hash::{Hash, Hasher};

/// Hard ceiling on entries rendered from `terminal.list` (matches `tessel-host` `MAX_TERMINALS`).
pub const MAX_LISTED_TERMINALS: usize = 64;
/// Hard ceiling on pending creation metadata records rendered (matches `host_terminal_list::MAX_PENDING`).
pub const MAX_PENDING_CREATIONS: usize = 64;
/// Maximum rows rendered on a single page so frame work stays O(1) even at full daemon capacity.
pub const PAGE_SIZE: usize = 16;
/// Maximum Unicode scalar count rendered for a terminal or pending-creation identifier (display only).
pub const MAX_TERMINAL_ID_DISPLAY_CHARS: usize = 24;
/// Maximum Unicode scalar count rendered for an epoch identifier in informational rows (display only).
pub const MAX_EPOCH_DISPLAY_CHARS: usize = 16;

const PRIMARY_DARK_FG: Color32 = Color32::from_rgb(0x0b, 0x15, 0x30);
const WARN_TEXT_DARK: Color32 = Color32::from_rgb(0xff, 0xd9, 0x8f);
const DANGER_TEXT_DARK: Color32 = Color32::from_rgb(0xff, 0xb4, 0xb6);

/// Exit status of a listed remote terminal still lingering on the daemon (`CLOSED_LINGER`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExistingTerminalExit {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

/// Whether local `LeaseStore` holds a saved lease for a listed terminal (`host_terminal_list::Saved`).
///
/// Never carries a `LeaseKey`; when the store could not be read (`Unknown`), UI never claims
/// "no lease" or "controlled elsewhere" as proven lack of local access, forbids `ResumeSaved`,
/// and keeps read-only observation available.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SavedLease {
    Yes,
    No,
    Unknown,
}

/// Honest reason why the local encrypted lease store could not be read during listing
/// (`host_terminal_list::IndexError`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LeaseStoreError {
    /// Another Tessel window currently holds the local lease store lock.
    Locked,
    /// Local lease store could not be decoded and was left untouched on disk.
    Corrupt,
    /// Local lease store is unavailable in the current environment.
    Unavailable,
}

/// Read-only metadata for a locally recorded pending creation (`host_terminal_list::PendingRow`).
///
/// Never carries a deduplication key or `LeaseKey`, and never triggers automatic creation or cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PendingCreationEntry<'a> {
    /// Raw pending-creation record identifier (sanitized strictly for display).
    pub id: &'a str,
    /// Raw protocol `Epoch` on which the creation was recorded (sanitized strictly for display).
    pub epoch: &'a str,
    /// `true` when `epoch == list_epoch` (current host daemon session); `false` when recorded
    /// against an earlier host session.
    pub current: bool,
}

/// Summary row for one remote terminal returned by `terminal.list`, enriched with non-secret local
/// desktop state (`saved_lease`, `open_locally`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExistingTerminalEntry<'a> {
    /// Exact raw `TerminalId` from `tessel-host-protocol` (never mutated in emitted actions).
    pub terminal_id: &'a str,
    pub pid: u32,
    /// Terminal `(rows, cols)` reported by `tessel-host`.
    pub size: (u16, u16),
    /// Number of currently attached client streams on `tessel-host`.
    pub attached: u32,
    /// Whether `tessel-host` reports an active writer lease on this terminal.
    pub leased_on_host: bool,
    /// Replay buffer bytes currently retained by `tessel-host` for this terminal.
    pub retained_bytes: u64,
    /// `Some` when the remote process has already exited and is lingering briefly.
    pub exited: Option<ExistingTerminalExit>,
    /// Tri-state local lease knowledge (`Yes`, `No`, `Unknown`).
    pub saved_lease: SavedLease,
    /// True when this terminal is already open in a local Tessel pane.
    pub open_locally: bool,
}

/// Fixed, translated failure reasons when listing remote terminals (`terminal.list`).
///
/// Deliberately carries no arbitrary remote string payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExistingTerminalsError {
    /// Host client is not currently connected.
    NotConnected,
    /// `terminal.list` timed out before answering.
    TimedOut,
    /// Host daemon restarted (`EpochChanged`) while listing.
    EpochChanged,
    /// Connected daemon does not support `terminal.list`.
    Unsupported,
    /// Transport or protocol refusal while listing.
    RequestFailed,
    /// Local list-worker pool is busy or could not spawn a worker (`host_terminal_list::Refused`).
    LocalBusy,
}

/// Current state of the remote terminal list for a host.
#[derive(Clone, Copy, Debug)]
pub enum ExistingTerminalsState<'a> {
    /// `terminal.list` request is in flight.
    Loading,
    /// `terminal.list` succeeded for `list_epoch` (exact raw protocol `Epoch` string).
    Loaded {
        list_epoch: &'a str,
        entries: &'a [ExistingTerminalEntry<'a>],
        /// True when the daemon reported more than [`MAX_LISTED_TERMINALS`] terminals.
        truncated: bool,
        /// `Some` when the local lease store could not be read (`entries` carry `SavedLease::Unknown`).
        store_error: Option<LeaseStoreError>,
        /// Informational pending-creation records for this host alias (never `dedup_key`).
        pending: &'a [PendingCreationEntry<'a>],
        /// True when local pending-creation records exceeded [`MAX_PENDING_CREATIONS`].
        pending_truncated: bool,
    },
    /// `terminal.list` failed with a fixed translated reason.
    Error { reason: ExistingTerminalsError },
}

/// Borrowed frame input for [`HostExistingTerminalsPanel`].
#[derive(Clone, Copy, Debug)]
pub struct ExistingTerminalsInput<'a> {
    pub id_salt: Id,
    pub host_label: &'a str,
    /// Current live daemon `Epoch` (exact raw protocol string; `None` while disconnected).
    pub live_epoch: Option<&'a str>,
    pub state: ExistingTerminalsState<'a>,
    /// True when local capacity and connection state allow attaching/resuming a remote terminal.
    pub can_attach: bool,
    /// True when the caller allows explicitly opening a brand-new terminal from a valid loaded
    /// list (never triggered automatically and never offered on `Error` or stale `epoch`).
    pub can_open_new: bool,
    pub french: bool,
}

/// Explicit user decision emitted by [`HostExistingTerminalsPanel`].
///
/// `epoch` and `terminal_id` always preserve the **exact raw protocol strings** passed in
/// [`ExistingTerminalsState::Loaded`] and [`ExistingTerminalEntry::terminal_id`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExistingTerminalsAction {
    /// Attach as a non-controlling observer (`Target::Existing { resume: false }`).
    AttachObserver { epoch: String, terminal_id: String },
    /// Resume with the locally saved lease (`Target::Existing { resume: true }`).
    ResumeSaved { epoch: String, terminal_id: String },
    /// Focus the local pane already displaying this terminal (only when `epoch` is still live).
    FocusLocalTerminal { epoch: String, terminal_id: String },
    /// Re-run `terminal.list` on the host.
    Refresh,
    /// Explicitly request creating a brand-new remote terminal (`Target::New`).
    OpenNewTerminal,
    /// Close the picker without spawning or attaching anything.
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModalBtnStyle {
    Primary,
    Standard,
}

/// Stateful, pure-UI controller for the existing remote terminals modal picker.
#[derive(Debug, Default)]
pub struct HostExistingTerminalsPanel {
    selected_idx: usize,
    page: usize,
    last_list_fingerprint: Option<u64>,
    escape_held: bool,
    last_modal_rect: Option<Rect>,
}

impl HostExistingTerminalsPanel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets selection and pagination when opening the modal for a new request.
    pub fn reset_selection(&mut self) {
        self.selected_idx = 0;
        self.page = 0;
        self.last_list_fingerprint = None;
    }

    #[must_use]
    pub const fn selected_index(&self) -> usize {
        self.selected_idx
    }

    #[must_use]
    pub const fn current_page(&self) -> usize {
        self.page
    }

    /// True while `Escape` remains physically held after dismissing the modal or an inner popup,
    /// allowing the caller to block `\x1b` autorepeat leakage into an underlying `Live` terminal.
    #[must_use]
    pub const fn is_escape_held(&self) -> bool {
        self.escape_held
    }

    #[must_use]
    pub const fn last_modal_rect(&self) -> Option<Rect> {
        self.last_modal_rect
    }

    /// Pre-terminal `Escape` filter:
    /// - Yields the first `Escape` press to any open `egui::Popup` without closing the modal.
    /// - Consumes `Escape` to emit [`ExistingTerminalsAction::Close`] when `modal_open` is `true`,
    ///   keeping `escape_held` latched until physical key release so held autorepeats never leak.
    pub fn handle_escape(
        &mut self,
        ctx: &egui::Context,
        modal_open: bool,
    ) -> Option<ExistingTerminalsAction> {
        let down = ctx.input(|i| i.key_down(Key::Escape));
        if !down {
            self.escape_held = false;
        }
        if modal_open && egui::Popup::is_any_open(ctx) {
            if down {
                self.escape_held = true;
            }
            return None;
        }
        if self.escape_held {
            consume_all_escape_events(ctx);
            return None;
        }
        if !modal_open {
            return None;
        }
        let pressed = ctx.input(|i| {
            i.focused
                && i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        Event::Key {
                            key: Key::Escape,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if modifiers.matches_logically(Modifiers::NONE)
                    )
                })
        });
        if pressed {
            consume_all_escape_events(ctx);
            self.escape_held = down;
            return Some(ExistingTerminalsAction::Close);
        }
        None
    }

    /// Renders the modal backdrop and existing-terminals picker card over `ctx`, clamping geometry
    /// inside `content_rect` so narrow (`280×320`) and standard (`800×500`) viewports never
    /// overflow.
    pub fn show_modal(
        &mut self,
        ctx: &egui::Context,
        input: &ExistingTerminalsInput<'_>,
    ) -> Option<ExistingTerminalsAction> {
        if let Some(esc_action) = self.handle_escape(ctx, true) {
            self.last_modal_rect = None;
            return Some(esc_action);
        }

        let screen = ctx.content_rect();
        if !screen.is_positive() {
            self.last_modal_rect = None;
            return None;
        }

        let pal = active_palette_from_ctx(ctx);
        let fr = input.french;
        let host = sanitized_host_or_fallback(input.host_label, fr);
        let modal_id = input.id_salt.with("existing-terminals-modal");
        let backdrop_id = modal_id.with("backdrop");
        let card_id = modal_id.with("card");
        let mut emitted: Option<ExistingTerminalsAction> = None;

        // 1. Full-window backdrop on `Order::Foreground` (pinned strictly below `card` via
        //    `set_sublayer`), so it also dims and intercepts clicks above `Order::Foreground`
        //    surfaces like `Settings` when opened from `Settings > SSH`.
        let mut backdrop_clicked = false;
        Area::new(backdrop_id)
            .order(Order::Foreground)
            .movable(false)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, CornerRadius::ZERO, pal.backdrop);
                if resp.clicked() {
                    backdrop_clicked = true;
                }
            });

        // 2. Sync pagination/selection state & process list keyboard navigation.
        if let ExistingTerminalsState::Loaded {
            list_epoch,
            entries,
            ..
        } = input.state
        {
            let bounded = clamp_entries(entries);
            let fp = fingerprint_list(list_epoch, bounded);
            if self.last_list_fingerprint != Some(fp) {
                self.last_list_fingerprint = Some(fp);
                if self.selected_idx >= bounded.len() {
                    self.selected_idx = bounded.len().saturating_sub(1);
                }
                let max_page = total_pages(bounded.len()).saturating_sub(1);
                if self.page > max_page {
                    self.page = max_page;
                }
            }
            let epoch_valid = is_epoch_live(list_epoch, input.live_epoch);
            if let Some(nav_action) =
                self.handle_loaded_keys(ctx, list_epoch, bounded, epoch_valid, input.can_attach)
            {
                emitted = Some(nav_action);
            }
        } else {
            self.last_list_fingerprint = None;
            self.selected_idx = 0;
            self.page = 0;
            // Consume bare `Enter` in non-loaded states only? No — leave unconsumed, and never
            // trigger `OpenNewTerminal` or `Refresh` implicitly on `Enter`.
        }

        // 3. Centered modal card clamped strictly inside `screen`.
        let margin = if screen.width() < 340.0 { 8.0 } else { 20.0 };
        let card_w = (screen.width() - margin * 2.0)
            .clamp(48.0, 660.0)
            .min(screen.width());
        let max_card_h = (screen.height() - margin * 2.0)
            .clamp(64.0, 520.0)
            .min(screen.height());

        ctx.set_sublayer(
            egui::LayerId::new(Order::Foreground, backdrop_id),
            egui::LayerId::new(Order::Foreground, card_id),
        );
        let area_resp = Area::new(card_id)
            .order(Order::Foreground)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .constrain_to(screen)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.surface)
                    .stroke(Stroke::new(1.0_f32, pal.border_strong))
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(if card_w < 300.0 { 8 } else { 14 }))
                    .show(ui, |ui| {
                        let inner_pad = if card_w < 300.0 { 16.0 } else { 28.0 };
                        let inner_w = (card_w - inner_pad - 2.0).max(32.0);
                        let inner_max_h = (max_card_h - inner_pad - 2.0).max(48.0);
                        ui.set_width(inner_w);
                        ui.set_max_height(inner_max_h);

                        if let Some(act) = self.render_card_contents(
                            ui,
                            modal_id,
                            &host,
                            input,
                            inner_w,
                            inner_max_h,
                            &pal,
                        ) && emitted.is_none()
                        {
                            emitted = Some(act);
                        }
                    });
            });

        let card_rect = area_resp.response.rect;
        self.last_modal_rect = Some(card_rect);

        if backdrop_clicked
            && ctx
                .input(|i| i.pointer.interact_pos())
                .is_none_or(|pos| !card_rect.contains(pos))
            && emitted.is_none()
        {
            emitted = Some(ExistingTerminalsAction::Close);
        }

        emitted
    }

    fn handle_loaded_keys(
        &mut self,
        ctx: &egui::Context,
        list_epoch: &str,
        bounded: &[ExistingTerminalEntry<'_>],
        epoch_valid: bool,
        can_attach: bool,
    ) -> Option<ExistingTerminalsAction> {
        if bounded.is_empty() || egui::Popup::is_any_open(ctx) {
            return None;
        }
        let len = bounded.len();
        let mut moved = false;

        ctx.input_mut(|i| {
            if !i.focused {
                return;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                self.selected_idx = (self.selected_idx + 1).min(len - 1);
                moved = true;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                self.selected_idx = self.selected_idx.saturating_sub(1);
                moved = true;
            }
            if i.consume_key(Modifiers::NONE, Key::PageDown) {
                self.selected_idx = (self.selected_idx + PAGE_SIZE).min(len - 1);
                moved = true;
            }
            if i.consume_key(Modifiers::NONE, Key::PageUp) {
                self.selected_idx = self.selected_idx.saturating_sub(PAGE_SIZE);
                moved = true;
            }
            if i.consume_key(Modifiers::NONE, Key::Home) {
                self.selected_idx = 0;
                moved = true;
            }
            if i.consume_key(Modifiers::NONE, Key::End) {
                self.selected_idx = len - 1;
                moved = true;
            }
        });

        if moved {
            self.page = self.selected_idx / PAGE_SIZE;
        }

        // If an explicit button currently holds egui focus, let egui handle `Enter` on that button;
        // otherwise `Enter` activates the selected running row's primary safe action when valid.
        if ctx.memory(|m| m.focused().is_some()) {
            return None;
        }
        if !epoch_valid {
            return None;
        }

        let enter_pressed = ctx.input(|i| {
            i.focused
                && i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        Event::Key {
                            key: Key::Enter,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if modifiers.matches_logically(Modifiers::NONE)
                    )
                })
        });
        if !enter_pressed {
            return None;
        }

        let entry = &bounded[self.selected_idx.min(len - 1)];
        let action = default_enter_action_for_row(list_epoch, entry, epoch_valid, can_attach)?;
        let _ = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        Some(action)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_card_contents(
        &mut self,
        ui: &mut Ui,
        modal_id: Id,
        host: &str,
        input: &ExistingTerminalsInput<'_>,
        inner_w: f32,
        inner_max_h: f32,
        pal: &UiPalette,
    ) -> Option<ExistingTerminalsAction> {
        let fr = input.french;
        let mut emitted: Option<ExistingTerminalsAction> = None;

        // Header row: icon + truncated title + optional Refresh + Close `×`
        let refresh_lbl = if fr { "Actualiser" } else { "Refresh" };
        let close_lbl = if fr { "Fermer" } else { "Close" };
        let hdr_close_a11y = if fr {
            "Fermer la fenêtre"
        } else {
            "Close dialog"
        };
        let loaded_epoch_valid = match input.state {
            ExistingTerminalsState::Loaded { list_epoch, .. } => {
                is_epoch_live(list_epoch, input.live_epoch)
            }
            ExistingTerminalsState::Loading | ExistingTerminalsState::Error { .. } => false,
        };
        let show_header_refresh =
            matches!(input.state, ExistingTerminalsState::Loaded { .. }) && inner_w >= 320.0;
        let top_y = ui.cursor().top();

        ui.horizontal(|ui| {
            let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            icons::paint(ui.painter(), icon_rect, Icon::Terminal, pal.accent);

            let title = if fr {
                format!("Terminaux existants — {host}")
            } else {
                format!("Existing terminals — {host}")
            };
            let refresh_reserve = if show_header_refresh {
                measure_btn_w(ui, refresh_lbl, true) + 8.0
            } else {
                0.0
            };
            let title_max_w = (ui.available_width() - 32.0 - refresh_reserve).max(24.0);
            ui.allocate_ui_with_layout(
                Vec2::new(title_max_w, 24.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(title)
                                .size(13.5)
                                .strong()
                                .color(pal.text_strong),
                        )
                        .truncate(),
                    );
                },
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let close_resp =
                    paint_icon_close_btn(ui, modal_id.with("hdr-close"), hdr_close_a11y, pal);
                if close_resp.clicked() && emitted.is_none() {
                    emitted = Some(ExistingTerminalsAction::Close);
                }
                if show_header_refresh {
                    let hdr_refresh_style = if loaded_epoch_valid {
                        ModalBtnStyle::Standard
                    } else {
                        ModalBtnStyle::Primary
                    };
                    let r_resp = paint_modal_btn(
                        ui,
                        modal_id.with("hdr-refresh"),
                        refresh_lbl,
                        None,
                        hdr_refresh_style,
                        true,
                        pal,
                    );
                    if r_resp.clicked() && emitted.is_none() {
                        emitted = Some(ExistingTerminalsAction::Refresh);
                    }
                }
            });
        });

        ui.add_space(5.0);
        paint_divider(ui, pal.border);
        ui.add_space(6.0);
        let used_header_h = (ui.cursor().top() - top_y).max(0.0);

        match input.state {
            ExistingTerminalsState::Loading => {
                let body_max_h = (inner_max_h - used_header_h - 42.0).max(40.0);
                egui::ScrollArea::vertical()
                    .id_salt(modal_id.with("loading-scroll"))
                    .auto_shrink([false, true])
                    .max_height(body_max_h)
                    .show(ui, |ui| {
                        let msg = if fr {
                            format!("Chargement des terminaux existants sur {host}…")
                        } else {
                            format!("Loading existing terminals on {host}…")
                        };
                        paint_notice_box(ui, &msg, false, false, pal);
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.set_width(inner_w);
                    ui.set_height(28.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let cancel_lbl = if fr { "Annuler" } else { "Cancel" };
                        if paint_modal_btn(
                            ui,
                            modal_id.with("loading-cancel"),
                            cancel_lbl,
                            None,
                            ModalBtnStyle::Standard,
                            false,
                            pal,
                        )
                        .clicked()
                            && emitted.is_none()
                        {
                            emitted = Some(ExistingTerminalsAction::Close);
                        }
                    });
                });
            }
            ExistingTerminalsState::Error { reason } => {
                let footer_reserve = if inner_w < 300.0 { 68.0 } else { 42.0 };
                let body_max_h = (inner_max_h - used_header_h - footer_reserve).max(40.0);
                egui::ScrollArea::vertical()
                    .id_salt(modal_id.with("error-scroll"))
                    .auto_shrink([false, true])
                    .max_height(body_max_h)
                    .show(ui, |ui| {
                        let msg = format_error_message(reason, host, fr);
                        paint_notice_box(ui, &msg, true, false, pal);
                    });
                ui.add_space(8.0);
                ui.set_max_width(inner_w);
                ui.horizontal_wrapped(|ui| {
                    if paint_modal_btn(
                        ui,
                        modal_id.with("err-refresh"),
                        refresh_lbl,
                        None,
                        ModalBtnStyle::Primary,
                        false,
                        pal,
                    )
                    .clicked()
                        && emitted.is_none()
                    {
                        emitted = Some(ExistingTerminalsAction::Refresh);
                    }
                    if paint_modal_btn(
                        ui,
                        modal_id.with("err-close"),
                        close_lbl,
                        None,
                        ModalBtnStyle::Standard,
                        false,
                        pal,
                    )
                    .clicked()
                        && emitted.is_none()
                    {
                        emitted = Some(ExistingTerminalsAction::Close);
                    }
                });
            }
            ExistingTerminalsState::Loaded {
                list_epoch,
                entries,
                truncated,
                store_error,
                pending,
                pending_truncated,
            } => {
                let epoch_valid = loaded_epoch_valid;
                let is_truncated = truncated || entries.len() > MAX_LISTED_TERMINALS;
                let bounded = clamp_entries(entries);
                let bounded_pending = clamp_pending(pending);
                let is_pending_truncated =
                    pending_truncated || pending.len() > MAX_PENDING_CREATIONS;
                let pages = total_pages(bounded.len());
                let cur_page = self.page.min(pages - 1);
                self.page = cur_page;

                let footer_reserve = if inner_w < 420.0 { 74.0 } else { 46.0 };
                let body_max_h = (inner_max_h - used_header_h - footer_reserve).clamp(44.0, 380.0);

                egui::ScrollArea::vertical()
                    .id_salt(modal_id.with(("rows-scroll", cur_page)))
                    .auto_shrink([false, true])
                    .max_height(body_max_h)
                    .show(ui, |ui| {
                        if !epoch_valid {
                            let stale_msg = if input.live_epoch.is_some() {
                                if fr {
                                    format!(
                                        "Le service hôte distant a redémarré sur {host} — cette liste provient d'une exécution précédente ; actualisez pour voir les terminaux actuels."
                                    )
                                } else {
                                    format!(
                                        "Remote host service restarted on {host} — this list is from an earlier daemon run; refresh to inspect current terminals."
                                    )
                                }
                            } else if fr {
                                format!(
                                    "La connexion à {host} est interrompue — reconnectez-vous ou actualisez avant de rattacher un terminal listé."
                                )
                            } else {
                                format!(
                                    "Connection to {host} is interrupted — reconnect or refresh before attaching to a listed terminal."
                                )
                            };
                            paint_notice_box(ui, &stale_msg, true, false, pal);
                            ui.add_space(6.0);
                        } else if !input.can_attach && bounded.iter().any(|e| !e.open_locally) {
                            let cap_msg = if fr {
                                "Fermez un autre volet de terminal distant localement pour rattacher ou reprendre une session de cette liste."
                            } else {
                                "Close another remote terminal pane locally to attach or resume a session from this list."
                            };
                            paint_notice_box(ui, cap_msg, false, true, pal);
                            ui.add_space(6.0);
                        }

                        if let Some(store_err) = store_error {
                            let (store_msg, is_danger) = format_store_error_notice(store_err, fr);
                            paint_notice_box(ui, store_msg, is_danger, !is_danger, pal);
                            ui.add_space(6.0);
                        }

                        if is_truncated {
                            let trunc_msg = if fr {
                                format!(
                                    "Affichage limité aux {MAX_LISTED_TERMINALS} premiers terminaux signalés par l'hôte."
                                )
                            } else {
                                format!(
                                    "Showing the first {MAX_LISTED_TERMINALS} terminals reported by the host."
                                )
                            };
                            paint_notice_box(ui, &trunc_msg, false, true, pal);
                            ui.add_space(6.0);
                        }

                        if bounded.is_empty() {
                            let empty_msg = if fr {
                                format!("Aucun terminal existant n'est actif sur {host}.")
                            } else {
                                format!("No existing terminals are running on {host}.")
                            };
                            paint_notice_box(ui, &empty_msg, false, false, pal);
                        } else {
                            let page_start = cur_page * PAGE_SIZE;
                            let page_end = (page_start + PAGE_SIZE).min(bounded.len());
                            for (offset, entry) in bounded[page_start..page_end].iter().enumerate()
                            {
                                let abs_idx = page_start + offset;
                                let selected = abs_idx == self.selected_idx;
                                if let Some(row_act) = render_terminal_row(
                                    ui,
                                    modal_id.with(("row", abs_idx)),
                                    list_epoch,
                                    entry,
                                    selected,
                                    epoch_valid,
                                    input.can_attach,
                                    fr,
                                    pal,
                                    &mut self.selected_idx,
                                    abs_idx,
                                ) && emitted.is_none()
                                {
                                    emitted = Some(row_act);
                                }
                                if offset + 1 < page_end - page_start {
                                    ui.add_space(4.0);
                                }
                            }
                        }

                        if !bounded_pending.is_empty() || is_pending_truncated {
                            render_pending_creations_section(
                                ui,
                                bounded_pending,
                                is_pending_truncated,
                                fr,
                                pal,
                            );
                        }
                    });

                ui.add_space(6.0);
                paint_divider(ui, pal.border);
                ui.add_space(6.0);

                // Footer: pagination (left) + explicit actions (right)
                let offer_open_new = input.can_open_new && epoch_valid;
                ui.set_max_width(inner_w);
                ui.horizontal_wrapped(|ui| {
                    if pages > 1 {
                        let prev_lbl = if fr { "Précédent" } else { "Previous" };
                        let next_lbl = if fr { "Suivant" } else { "Next" };
                        if self.page > 0
                            && paint_modal_btn(
                                ui,
                                modal_id.with("page-prev"),
                                prev_lbl,
                                None,
                                ModalBtnStyle::Standard,
                                true,
                                pal,
                            )
                            .clicked()
                        {
                            self.page -= 1;
                            self.selected_idx = self.page * PAGE_SIZE;
                            ui.ctx().request_repaint();
                        }
                        let page_txt = if fr {
                            format!("Page {} sur {pages}", self.page + 1)
                        } else {
                            format!("Page {} of {pages}", self.page + 1)
                        };
                        ui.label(egui::RichText::new(page_txt).size(11.5).color(pal.text_dim));
                        if self.page + 1 < pages
                            && paint_modal_btn(
                                ui,
                                modal_id.with("page-next"),
                                next_lbl,
                                None,
                                ModalBtnStyle::Standard,
                                true,
                                pal,
                            )
                            .clicked()
                        {
                            self.page += 1;
                            self.selected_idx = self.page * PAGE_SIZE;
                            ui.ctx().request_repaint();
                        }
                    }

                    if !show_header_refresh {
                        let ftr_refresh_style = if epoch_valid {
                            ModalBtnStyle::Standard
                        } else {
                            ModalBtnStyle::Primary
                        };
                        if paint_modal_btn(
                            ui,
                            modal_id.with("ftr-refresh"),
                            refresh_lbl,
                            None,
                            ftr_refresh_style,
                            true,
                            pal,
                        )
                        .clicked()
                            && emitted.is_none()
                        {
                            emitted = Some(ExistingTerminalsAction::Refresh);
                        }
                    }

                    if offer_open_new {
                        let new_lbl = if fr {
                            "Nouveau terminal"
                        } else {
                            "New terminal"
                        };
                        if paint_modal_btn(
                            ui,
                            modal_id.with("ftr-open-new"),
                            new_lbl,
                            None,
                            ModalBtnStyle::Standard,
                            true,
                            pal,
                        )
                        .clicked()
                            && emitted.is_none()
                        {
                            emitted = Some(ExistingTerminalsAction::OpenNewTerminal);
                        }
                    }

                    if paint_modal_btn(
                        ui,
                        modal_id.with("ftr-close"),
                        close_lbl,
                        None,
                        ModalBtnStyle::Standard,
                        true,
                        pal,
                    )
                    .clicked()
                        && emitted.is_none()
                    {
                        emitted = Some(ExistingTerminalsAction::Close);
                    }
                });
            }
        }

        emitted
    }
}

fn clamp_entries<'a>(entries: &'a [ExistingTerminalEntry<'a>]) -> &'a [ExistingTerminalEntry<'a>] {
    let cap = entries.len().min(MAX_LISTED_TERMINALS);
    &entries[..cap]
}

fn clamp_pending<'a>(pending: &'a [PendingCreationEntry<'a>]) -> &'a [PendingCreationEntry<'a>] {
    let cap = pending.len().min(MAX_PENDING_CREATIONS);
    &pending[..cap]
}

const fn total_pages(len: usize) -> usize {
    if len == 0 { 1 } else { len.div_ceil(PAGE_SIZE) }
}

fn is_epoch_live(list_epoch: &str, live_epoch: Option<&str>) -> bool {
    !list_epoch.is_empty() && live_epoch == Some(list_epoch)
}

fn fingerprint_list(list_epoch: &str, entries: &[ExistingTerminalEntry<'_>]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    list_epoch.hash(&mut hasher);
    entries.len().hash(&mut hasher);
    for entry in entries {
        entry.terminal_id.hash(&mut hasher);
        entry.pid.hash(&mut hasher);
        entry.leased_on_host.hash(&mut hasher);
        entry.exited.hash(&mut hasher);
        entry.saved_lease.hash(&mut hasher);
        entry.open_locally.hash(&mut hasher);
    }
    hasher.finish()
}

fn default_enter_action_for_row(
    list_epoch: &str,
    entry: &ExistingTerminalEntry<'_>,
    epoch_valid: bool,
    can_attach: bool,
) -> Option<ExistingTerminalsAction> {
    if !epoch_valid {
        return None;
    }
    if entry.open_locally {
        return Some(ExistingTerminalsAction::FocusLocalTerminal {
            epoch: list_epoch.to_owned(),
            terminal_id: entry.terminal_id.to_owned(),
        });
    }
    if entry.exited.is_some() || !can_attach {
        return None;
    }
    if entry.saved_lease == SavedLease::Yes {
        Some(ExistingTerminalsAction::ResumeSaved {
            epoch: list_epoch.to_owned(),
            terminal_id: entry.terminal_id.to_owned(),
        })
    } else {
        Some(ExistingTerminalsAction::AttachObserver {
            epoch: list_epoch.to_owned(),
            terminal_id: entry.terminal_id.to_owned(),
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn render_terminal_row(
    ui: &mut Ui,
    row_id: Id,
    list_epoch: &str,
    entry: &ExistingTerminalEntry<'_>,
    selected: bool,
    epoch_valid: bool,
    can_attach: bool,
    fr: bool,
    pal: &UiPalette,
    selected_idx_out: &mut usize,
    abs_idx: usize,
) -> Option<ExistingTerminalsAction> {
    let mut emitted = None;
    let clean_id = sanitize_host_text(entry.terminal_id, MAX_TERMINAL_ID_DISPLAY_CHARS);
    let display_id = if clean_id.is_empty() {
        "terminal".to_owned()
    } else {
        clean_id
    };

    let (status_badge, badge_tone) = format_row_badge(entry, fr);
    let (rows, cols) = entry.size;
    let retained_str = format_compact_bytes(entry.retained_bytes, fr);
    let meta = format_row_meta(entry.pid, rows, cols, entry.attached, &retained_str, fr);

    let bg = if selected {
        mix_color(pal.accent, pal.surface_2, 0.14)
    } else {
        pal.surface_2
    };
    let border = if selected {
        Stroke::new(1.0_f32, with_alpha(pal.accent, 0.65))
    } else {
        Stroke::new(1.0_f32, pal.border)
    };

    let row_inner_w = (ui.available_width() - 22.0).max(24.0);
    let mut btn_rects: [Option<Rect>; 2] = [None, None];
    let mut btn_count = 0_usize;

    let frame_resp = egui::Frame::new()
        .fill(bg)
        .stroke(border)
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_max_width(row_inner_w);
            ui.set_min_width(row_inner_w);
            ui.horizontal_wrapped(|ui| {
                let id_resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(format!("#{display_id}"))
                            .size(12.5)
                            .strong()
                            .color(pal.text_strong),
                    )
                    .wrap()
                    .sense(Sense::click()),
                );
                if id_resp.clicked() {
                    *selected_idx_out = abs_idx;
                }
                paint_badge_pill(ui, &status_badge, badge_tone, pal);
                if entry.open_locally {
                    let local_lbl = if fr { "Ouvert ici" } else { "Open here" };
                    paint_badge_pill(ui, local_lbl, BadgeTone::Accent, pal);
                } else if entry.exited.is_none() {
                    match entry.saved_lease {
                        SavedLease::Yes => {
                            let saved_lbl = if fr {
                                "Accès enregistré"
                            } else {
                                "Saved access"
                            };
                            paint_badge_pill(ui, saved_lbl, BadgeTone::Accent, pal);
                        }
                        SavedLease::Unknown => {
                            let unk_lbl = if fr {
                                "Accès inconnu"
                            } else {
                                "Access unknown"
                            };
                            paint_badge_pill(ui, unk_lbl, BadgeTone::Warn, pal);
                        }
                        SavedLease::No => {}
                    }
                }
            });

            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                let meta_resp = ui.add(
                    egui::Label::new(egui::RichText::new(meta).size(11.5).color(pal.text_dim))
                        .wrap()
                        .sense(Sense::click()),
                );
                if meta_resp.clicked() {
                    *selected_idx_out = abs_idx;
                }

                if epoch_valid {
                    if entry.open_locally {
                        let focus_lbl = if fr {
                            "Aller au volet"
                        } else {
                            "Switch to pane"
                        };
                        let focus_a11y = format!("{focus_lbl} #{display_id}");
                        let resp = paint_modal_btn(
                            ui,
                            row_id.with("focus-local"),
                            focus_lbl,
                            Some(&focus_a11y),
                            ModalBtnStyle::Primary,
                            true,
                            pal,
                        );
                        btn_rects[btn_count] = Some(resp.rect);
                        btn_count += 1;
                        if resp.clicked() {
                            *selected_idx_out = abs_idx;
                            emitted = Some(ExistingTerminalsAction::FocusLocalTerminal {
                                epoch: list_epoch.to_owned(),
                                terminal_id: entry.terminal_id.to_owned(),
                            });
                        }
                    } else if can_attach {
                        match entry.exited {
                            None => {
                                let has_saved = entry.saved_lease == SavedLease::Yes;
                                if has_saved {
                                    let resume_lbl = if fr { "Reprendre" } else { "Resume" };
                                    let resume_a11y = format!("{resume_lbl} #{display_id}");
                                    let resp = paint_modal_btn(
                                        ui,
                                        row_id.with("resume"),
                                        resume_lbl,
                                        Some(&resume_a11y),
                                        ModalBtnStyle::Primary,
                                        true,
                                        pal,
                                    );
                                    btn_rects[btn_count] = Some(resp.rect);
                                    btn_count += 1;
                                    if resp.clicked() {
                                        *selected_idx_out = abs_idx;
                                        emitted = Some(ExistingTerminalsAction::ResumeSaved {
                                            epoch: list_epoch.to_owned(),
                                            terminal_id: entry.terminal_id.to_owned(),
                                        });
                                    }
                                }
                                let obs_lbl = if fr { "Observer" } else { "Observe" };
                                let obs_a11y = format!("{obs_lbl} #{display_id}");
                                let obs_style = if has_saved {
                                    ModalBtnStyle::Standard
                                } else {
                                    ModalBtnStyle::Primary
                                };
                                let resp = paint_modal_btn(
                                    ui,
                                    row_id.with("observe"),
                                    obs_lbl,
                                    Some(&obs_a11y),
                                    obs_style,
                                    true,
                                    pal,
                                );
                                btn_rects[btn_count] = Some(resp.rect);
                                btn_count += 1;
                                if resp.clicked() {
                                    *selected_idx_out = abs_idx;
                                    emitted = Some(ExistingTerminalsAction::AttachObserver {
                                        epoch: list_epoch.to_owned(),
                                        terminal_id: entry.terminal_id.to_owned(),
                                    });
                                }
                            }
                            Some(_) => {
                                let final_lbl = if fr {
                                    "Voir la sortie finale"
                                } else {
                                    "View final output"
                                };
                                let final_a11y = format!("{final_lbl} #{display_id}");
                                let resp = paint_modal_btn(
                                    ui,
                                    row_id.with("observe-exited"),
                                    final_lbl,
                                    Some(&final_a11y),
                                    ModalBtnStyle::Standard,
                                    true,
                                    pal,
                                );
                                btn_rects[btn_count] = Some(resp.rect);
                                btn_count += 1;
                                if resp.clicked() {
                                    *selected_idx_out = abs_idx;
                                    emitted = Some(ExistingTerminalsAction::AttachObserver {
                                        epoch: list_epoch.to_owned(),
                                        terminal_id: entry.terminal_id.to_owned(),
                                    });
                                }
                            }
                        }
                    }
                }
            });
        });

    let _ = btn_count;
    let visible_row_rect = frame_resp.response.rect.intersect(ui.clip_rect());
    if ui.input(|i| i.pointer.primary_clicked())
        && let Some(pos) = ui.input(|i| i.pointer.interact_pos())
        && visible_row_rect.contains(pos)
        && !btn_rects.iter().flatten().any(|r| r.contains(pos))
    {
        *selected_idx_out = abs_idx;
    }

    emitted
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BadgeTone {
    Accent,
    Warn,
    Neutral,
}

fn format_row_badge(entry: &ExistingTerminalEntry<'_>, fr: bool) -> (String, BadgeTone) {
    if let Some(exit) = entry.exited {
        let label = match (exit.exit_code, exit.signal, fr) {
            (Some(c), _, false) => format!("Exited (code {c})"),
            (Some(c), _, true) => format!("Terminé (code {c})"),
            (None, Some(s), false) => format!("Exited (signal {s})"),
            (None, Some(s), true) => format!("Terminé (signal {s})"),
            (None, None, false) => "Exited".to_owned(),
            (None, None, true) => "Terminé".to_owned(),
        };
        return (label, BadgeTone::Warn);
    }
    if entry.leased_on_host && entry.saved_lease == SavedLease::No {
        let lbl = if fr {
            "Contrôlé ailleurs".to_owned()
        } else {
            "Controlled elsewhere".to_owned()
        };
        (lbl, BadgeTone::Warn)
    } else {
        let lbl = if fr {
            "Actif".to_owned()
        } else {
            "Running".to_owned()
        };
        (lbl, BadgeTone::Neutral)
    }
}

fn format_row_meta(
    pid: u32,
    rows: u16,
    cols: u16,
    attached: u32,
    retained: &str,
    fr: bool,
) -> String {
    if fr {
        format!("PID {pid} · {cols}×{rows} · {attached} rattaché(s) · tampon {retained}")
    } else {
        format!("PID {pid} · {cols}×{rows} · {attached} attached · buffer {retained}")
    }
}

fn format_compact_bytes(bytes: u64, fr: bool) -> String {
    if bytes < 1024 {
        return if fr {
            format!("{bytes} o")
        } else {
            format!("{bytes} B")
        };
    }
    let kb = (bytes as f64) / 1024.0;
    if kb < 1024.0 {
        let s = format!("{kb:.1}");
        return if fr {
            format!("{} Ko", s.replace('.', ","))
        } else {
            format!("{s} KB")
        };
    }
    let mb = kb / 1024.0;
    let s = format!("{mb:.1}");
    if fr {
        format!("{} Mo", s.replace('.', ","))
    } else {
        format!("{s} MB")
    }
}

fn format_error_message(reason: ExistingTerminalsError, host: &str, fr: bool) -> String {
    match (reason, fr) {
        (ExistingTerminalsError::NotConnected, false) => {
            format!("Could not reach {host} to list existing terminals.")
        }
        (ExistingTerminalsError::NotConnected, true) => {
            format!("Impossible de joindre {host} pour lister les terminaux existants.")
        }
        (ExistingTerminalsError::TimedOut, false) => {
            format!("Timed out waiting for {host} to list existing terminals.")
        }
        (ExistingTerminalsError::TimedOut, true) => {
            format!("Délai dépassé en attendant la liste des terminaux existants sur {host}.")
        }
        (ExistingTerminalsError::EpochChanged, false) => format!(
            "Remote host service restarted on {host} — refresh to list terminals from the current daemon."
        ),
        (ExistingTerminalsError::EpochChanged, true) => format!(
            "Le service hôte distant a redémarré sur {host} — actualisez pour lister les terminaux du service actuel."
        ),
        (ExistingTerminalsError::Unsupported, false) => {
            format!("Remote host service on {host} does not support listing terminals.")
        }
        (ExistingTerminalsError::Unsupported, true) => format!(
            "Le service hôte distant sur {host} ne prend pas en charge la liste des terminaux."
        ),
        (ExistingTerminalsError::RequestFailed, false) => {
            format!("Could not list existing terminals on {host} — refresh to try again.")
        }
        (ExistingTerminalsError::RequestFailed, true) => {
            format!(
                "Impossible de lister les terminaux existants sur {host} — actualisez pour réessayer."
            )
        }
        (ExistingTerminalsError::LocalBusy, false) => {
            "Too many terminal list refreshes are already running locally — try again in a moment."
                .to_owned()
        }
        (ExistingTerminalsError::LocalBusy, true) => {
            "Trop d'actualisations de listes de terminaux sont déjà en cours localement — réessayez dans un instant."
                .to_owned()
        }
    }
}

fn format_store_error_notice(err: LeaseStoreError, fr: bool) -> (&'static str, bool) {
    match (err, fr) {
        (LeaseStoreError::Locked, false) => (
            "Local access store is locked by another Tessel window — saved control access cannot be verified right now (read-only observation remains available).",
            false,
        ),
        (LeaseStoreError::Locked, true) => (
            "Le coffre d'accès local est verrouillé par une autre fenêtre Tessel — les accès enregistrés ne peuvent pas être vérifiés pour l'instant (l'observation en lecture seule reste disponible).",
            false,
        ),
        (LeaseStoreError::Corrupt, false) => (
            "Local access store could not be read and was left untouched — saved control access is unknown (read-only observation remains available).",
            true,
        ),
        (LeaseStoreError::Corrupt, true) => (
            "Le coffre d'accès local est illisible et n'a pas été modifié — les accès enregistrés sont inconnus (l'observation en lecture seule reste disponible).",
            true,
        ),
        (LeaseStoreError::Unavailable, false) => (
            "Local access store is unavailable in this environment — saved control access cannot be verified (read-only observation remains available).",
            false,
        ),
        (LeaseStoreError::Unavailable, true) => (
            "Le coffre d'accès local est indisponible dans cet environnement — les accès enregistrés ne peuvent pas être vérifiés (l'observation en lecture seule reste disponible).",
            false,
        ),
    }
}

fn render_pending_creations_section(
    ui: &mut Ui,
    pending: &[PendingCreationEntry<'_>],
    truncated: bool,
    fr: bool,
    pal: &UiPalette,
) {
    let bounded = clamp_pending(pending);
    ui.add_space(8.0);
    paint_divider(ui, pal.border);
    ui.add_space(6.0);

    let sec_w = ui.available_width().max(16.0);
    ui.allocate_ui_with_layout(
        Vec2::new(sec_w, 0.0),
        Layout::top_down(Align::Min),
        |ui| {
            ui.set_max_width(sec_w);
            let title = if fr {
                "Créations enregistrées"
            } else {
                "Recorded creations"
            };
            let note = if fr {
                "Créations locales non confirmées ou remplacées (informatif uniquement — vérifiez les terminaux listés ci-dessus avant d'en ouvrir un nouveau)."
            } else {
                "Unconfirmed or superseded local creation records (informational only — inspect listed terminals above before opening a new one)."
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(title)
                        .size(12.0)
                        .strong()
                        .color(pal.text_strong),
                )
                .wrap(),
            );
            ui.add_space(2.0);
            ui.add(egui::Label::new(egui::RichText::new(note).size(11.0).color(pal.text_dim)).wrap());
        },
    );

    if truncated {
        ui.add_space(4.0);
        let trunc_msg = if fr {
            format!(
                "Affichage limité aux {MAX_PENDING_CREATIONS} premières créations enregistrées."
            )
        } else {
            format!("Showing the first {MAX_PENDING_CREATIONS} recorded creations.")
        };
        paint_notice_box(ui, &trunc_msg, false, true, pal);
    }

    if !bounded.is_empty() {
        ui.add_space(4.0);
    }
    let row_inner_w = (ui.available_width() - 22.0).max(16.0);
    for (idx, item) in bounded.iter().enumerate() {
        let clean_id = sanitize_host_text(item.id, MAX_TERMINAL_ID_DISPLAY_CHARS);
        let display_id = if clean_id.is_empty() {
            "pending".to_owned()
        } else {
            clean_id
        };
        let clean_ep = sanitize_host_text(item.epoch, MAX_EPOCH_DISPLAY_CHARS);
        let display_ep = if clean_ep.is_empty() {
            "—".to_owned()
        } else {
            clean_ep
        };
        let (badge_lbl, badge_tone) = match (item.current, fr) {
            (true, false) => ("Creation to review", BadgeTone::Warn),
            (true, true) => ("Création à vérifier", BadgeTone::Warn),
            (false, false) => ("Previous host session", BadgeTone::Neutral),
            (false, true) => ("Session hôte antérieure", BadgeTone::Neutral),
        };
        let ep_meta = if fr {
            format!("session hôte {display_ep}")
        } else {
            format!("host session {display_ep}")
        };

        egui::Frame::new()
            .fill(pal.surface_2)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 5))
            .show(ui, |ui| {
                ui.set_max_width(row_inner_w);
                ui.set_min_width(row_inner_w);
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("#{display_id}"))
                                .size(11.5)
                                .strong()
                                .color(pal.text),
                        )
                        .wrap(),
                    );
                    paint_badge_pill(ui, badge_lbl, badge_tone, pal);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(ep_meta).size(11.0).color(pal.text_dim),
                        )
                        .wrap(),
                    );
                });
            });
        if idx + 1 < bounded.len() {
            ui.add_space(3.0);
        }
    }
}

fn sanitized_host_or_fallback(raw: &str, fr: bool) -> String {
    let clean = sanitize_host_text(raw, MAX_HOST_LABEL_CHARS);
    if clean.is_empty() {
        if fr {
            "l'hôte distant".to_owned()
        } else {
            "remote host".to_owned()
        }
    } else {
        clean
    }
}

fn paint_divider(ui: &mut Ui, color: Color32) {
    let w = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0_f32, color),
    );
}

fn paint_notice_box(ui: &mut Ui, text: &str, danger: bool, warn: bool, pal: &UiPalette) {
    let dark = ui.visuals().dark_mode;
    let (stroke_col, fg) = if danger {
        (
            with_alpha(pal.danger, 0.55),
            if dark { DANGER_TEXT_DARK } else { pal.danger },
        )
    } else if warn {
        (
            with_alpha(pal.warn, 0.55),
            if dark { WARN_TEXT_DARK } else { pal.warn },
        )
    } else {
        (pal.border, pal.text)
    };
    let box_inner_w = (ui.available_width() - 22.0).max(16.0);
    egui::Frame::new()
        .fill(pal.surface_2)
        .stroke(Stroke::new(1.0_f32, stroke_col))
        .corner_radius(CornerRadius::same(7))
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_max_width(box_inner_w);
            ui.set_min_width(box_inner_w);
            ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).color(fg)).wrap());
        });
}

fn paint_badge_pill(ui: &mut Ui, label: &str, tone: BadgeTone, pal: &UiPalette) {
    let dark = ui.visuals().dark_mode;
    let (bg, fg) = match tone {
        BadgeTone::Accent => (
            with_alpha(pal.accent, 0.18),
            if dark {
                brighten(pal.accent, 1.15)
            } else {
                pal.accent
            },
        ),
        BadgeTone::Warn => (
            with_alpha(pal.warn, 0.18),
            if dark { WARN_TEXT_DARK } else { pal.warn },
        ),
        BadgeTone::Neutral => (pal.surface_3, pal.text_dim),
    };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(10.5), fg);
    let desired_w = (galley.size().x + 12.0).ceil();
    let alloc_w = desired_w.min(ui.max_rect().width().floor().max(8.0));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(alloc_w, 17.0), Sense::hover());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, CornerRadius::same(4), bg);
    painter.galley(
        Pos2::new(
            (rect.center().x - galley.size().x * 0.5).max(rect.left() + 4.0),
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        fg,
    );
}

fn measure_btn_w(ui: &Ui, label: &str, small: bool) -> f32 {
    let font_px = if small { 11.5 } else { 12.0 };
    let pad_x = if small { 18.0 } else { 22.0 };
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(font_px),
        Color32::WHITE,
    );
    (galley.size().x + pad_x).ceil().max(48.0)
}

fn paint_modal_btn(
    ui: &mut Ui,
    id: Id,
    label: &str,
    a11y_label: Option<&str>,
    style: ModalBtnStyle,
    small: bool,
    pal: &UiPalette,
) -> Response {
    let h = if small { 24.0 } else { 28.0 };
    let w = measure_btn_w(ui, label, small).min(ui.max_rect().width().max(36.0));
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    let accessible = a11y_label.unwrap_or(label);
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, accessible));

    let hovered = resp.hovered();
    let (bg, border, fg) = match style {
        ModalBtnStyle::Primary => {
            let fill = if hovered {
                brighten(pal.accent, 1.08)
            } else {
                pal.accent
            };
            let [r, g, b, _] = fill.to_srgba_unmultiplied();
            let lum = 0.299 * (r as f32) + 0.587 * (g as f32) + 0.114 * (b as f32);
            let fg_col = if lum > 140.0 {
                PRIMARY_DARK_FG
            } else {
                Color32::WHITE
            };
            (fill, Color32::TRANSPARENT, fg_col)
        }
        ModalBtnStyle::Standard => (
            if hovered {
                pal.surface_3
            } else {
                Color32::TRANSPARENT
            },
            pal.border_strong,
            pal.text,
        ),
    };

    let painter: Painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect(
        rect,
        CornerRadius::same(6),
        bg,
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    if resp.has_focus() {
        painter.rect_stroke(
            rect.expand(1.0),
            CornerRadius::same(6),
            Stroke::new(2.0_f32, pal.accent),
            StrokeKind::Outside,
        );
    }
    let font_px = if small { 11.5 } else { 12.0 };
    let galley = painter.layout_no_wrap(label.to_owned(), FontId::proportional(font_px), fg);
    painter.galley(
        Pos2::new(
            (rect.center().x - galley.size().x * 0.5).max(rect.left() + 4.0),
            rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        fg,
    );
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

fn paint_icon_close_btn(ui: &mut Ui, id: Id, label: &str, pal: &UiPalette) -> Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(5), pal.surface_3);
    }
    if resp.has_focus() {
        painter.rect_stroke(
            rect.expand(1.0),
            CornerRadius::same(5),
            Stroke::new(2.0_f32, pal.accent),
            StrokeKind::Outside,
        );
    }
    let fg = if resp.hovered() {
        pal.text_strong
    } else {
        pal.text_dim
    };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(13.0));
    icons::paint(&painter, icon_rect, Icon::Close, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(label)
}

fn consume_all_escape_events(ctx: &egui::Context) {
    ctx.input_mut(|i| {
        let _ = i.consume_key(Modifiers::NONE, Key::Escape);
        i.events.retain(|ev| {
            !matches!(
                ev,
                Event::Key {
                    key: Key::Escape,
                    ..
                }
            )
        });
    });
}

fn active_palette_from_ctx(ctx: &egui::Context) -> UiPalette {
    let style = ctx.style();
    let v = &style.visuals;
    if v.dark_mode {
        let id = ctx
            .data(|d| d.get_temp::<ThemeId>(Id::new("tessel-theme")))
            .unwrap_or_default();
        id.spec().ui
    } else {
        UiPalette {
            chrome: v.extreme_bg_color,
            surface: v.panel_fill,
            surface_2: v.window_fill,
            surface_3: v.selection.bg_fill,
            term: v.faint_bg_color,
            backdrop: Color32::from_rgba_unmultiplied(0, 0, 0, 110),
            border: v.widgets.noninteractive.bg_stroke.color,
            border_strong: v.widgets.inactive.bg_stroke.color,
            accent: v.selection.stroke.color,
            warn: v.warn_fg_color,
            danger: v.error_fg_color,
            text: v.text_color(),
            text_strong: v.strong_text_color(),
            text_dim: v.weak_text_color(),
        }
    }
}

fn with_alpha(c: Color32, factor: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let na = ((a as f32) * factor.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(r, g, b, na)
}

fn brighten(c: Color32, factor: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    let scale = |v: u8| ((v as f32) * factor).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(scale(r), scale(g), scale(b), a)
}

fn mix_color(a: Color32, b: Color32, weight_a: f32) -> Color32 {
    let w = weight_a.clamp(0.0, 1.0);
    let [ar, ag, ab, aa] = a.to_srgba_unmultiplied();
    let [br, bg, bb, ba] = b.to_srgba_unmultiplied();
    let blend = |x: u8, y: u8| ((x as f32) * w + (y as f32) * (1.0 - w)).round() as u8;
    Color32::from_rgba_unmultiplied(blend(ar, br), blend(ag, bg), blend(ab, bb), blend(aa, ba))
}
