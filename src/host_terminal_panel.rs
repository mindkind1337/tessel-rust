//! Pure `egui` UI & state machine for remote `tessel-host` PTY panes (`TerminalPane.vue` parity).
//!
//! # Architectural & Security Boundaries
//! - **Zero I/O:** This module never opens sockets, spawns processes, touches persistence, or calls
//!   `tessel-host-client`. Every user decision is returned as an explicit [`HostTerminalAction`].
//! - **UI Routing Filter ≠ Authority:** [`HostTerminalPanel::accepts_input`] and
//!   [`HostTerminalPanel::accepts_resize`] are pure client-side UI event-routing filters used by
//!   the pane host to suppress keyboard/paste/IME writes, mouse-tracking reports, `\x1b[6n` DSR
//!   auto-replies, and PTY `resize(rows, cols)` outside [`HostTerminalPhase::Live`]. They are
//!   **not** security authority: `tessel-host` independently re-validates lease ownership and
//!   daemon epoch on every protocol frame.
//! - **Zero Layout Shift:** [`HostTerminalPanel::show_overlays`] paints floating `.root-note`
//!   banners (top) and `.exit-overlay` / close-choice cards (bottom) directly over an already
//!   allocated `terminal_rect` without calling `ui.allocate_*`, so terminal `(rows, cols)` never
//!   reflow or bounce when banners appear or dismiss.

use crate::icons::{self, Icon};
use crate::theme_palettes::{ThemeId, UiPalette};
use eframe::egui::{
    self, Color32, CornerRadius, CursorIcon, Event, FontId, Id, Key, Modifiers, Painter, Pos2,
    Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType,
};
use std::collections::BTreeSet;
use std::time::Duration;

/// Maximum Unicode scalar count rendered for a remote host label.
pub const MAX_HOST_LABEL_CHARS: usize = 96;
/// Maximum Unicode scalar count retained for an opaque `tessel-host` epoch identifier.
pub const MAX_EPOCH_CHARS: usize = 128;
/// Maximum Unicode scalar count rendered for diagnostic notes or transport error details.
pub const MAX_NOTE_CHARS: usize = 240;
/// Hard ceiling on source Unicode scalars inspected by [`sanitize_host_text`] so hostile
/// multi-megabyte or control-only payloads remain O(1) in both CPU and heap allocation.
pub const MAX_RAW_SCAN_CHARS: usize = 4096;

const MAX_DISMISSED_KEYS: usize = 32;
const READONLY_FLASH_COOLDOWN_SECS: f64 = 4.0;
const READONLY_FLASH_DURATION_SECS: f64 = 1.6;

const TOP_BANNER_OFFSET_Y: f32 = 6.0;
const TOP_BANNER_GAP_Y: f32 = 8.0;
const BOTTOM_OVERLAY_OFFSET_Y: f32 = 18.0;
const EXIT_BTN_HEIGHT: f32 = 28.0;
const SMALL_BTN_HEIGHT: f32 = 24.0;
const DISMISS_BTN_SIDE: f32 = 22.0;

const PRIMARY_DARK_FG: Color32 = Color32::from_rgb(0x0b, 0x15, 0x30);
const WARN_TEXT_DARK: Color32 = Color32::from_rgb(0xff, 0xd9, 0x8f);
const DANGER_TEXT_DARK: Color32 = Color32::from_rgb(0xff, 0xb4, 0xb6);

/// Strips C0/DEL/C1 control characters and Unicode bidirectional formatting overrides from
/// host-supplied or transport-supplied strings, collapsing tabs/newlines into single ASCII spaces
/// and bounding both scanned input (`MAX_RAW_SCAN_CHARS`) and collected output (`max_chars`)
/// on the fly without allocating the full input length.
#[must_use]
pub fn sanitize_host_text(raw: &str, max_chars: usize) -> String {
    if max_chars == 0 || raw.is_empty() {
        return String::new();
    }
    let cap_chars = max_chars.min(MAX_RAW_SCAN_CHARS);
    let mut cleaned = String::new();
    let mut kept_chars = 0_usize;
    let mut prev_space = true;
    let mut overflowed = false;

    for ch in raw.chars().take(MAX_RAW_SCAN_CHARS) {
        if matches!(ch, '\t' | '\n' | '\r' | ' ') {
            if !prev_space {
                if kept_chars <= cap_chars {
                    cleaned.push(' ');
                    kept_chars += 1;
                }
                prev_space = true;
            }
            continue;
        }
        if is_forbidden_control_or_bidi(ch) {
            continue;
        }
        if cleaned.is_empty() {
            cleaned.reserve(cap_chars.saturating_add(2).min(256).saturating_mul(4));
        }
        if kept_chars >= cap_chars {
            overflowed = true;
            break;
        }
        cleaned.push(ch);
        kept_chars += 1;
        prev_space = false;
    }

    if !overflowed && cleaned.ends_with(' ') {
        cleaned.pop();
        kept_chars = kept_chars.saturating_sub(1);
    }
    if !overflowed && kept_chars <= max_chars {
        return cleaned;
    }
    if max_chars == 1 {
        return "…".to_owned();
    }
    while kept_chars > max_chars - 1 {
        if cleaned.pop().is_some() {
            kept_chars -= 1;
        } else {
            break;
        }
    }
    if cleaned.ends_with(' ') {
        cleaned.pop();
    }
    cleaned.push('…');
    cleaned
}

fn normalize_epoch_key(raw: &str) -> String {
    let mut key = String::new();
    let mut kept = 0_usize;
    for ch in raw.chars().take(MAX_RAW_SCAN_CHARS) {
        if is_forbidden_control_or_bidi(ch) || matches!(ch, '\t' | '\n' | '\r') {
            continue;
        }
        if key.is_empty() {
            key.reserve(raw.len().min(MAX_EPOCH_CHARS));
        }
        key.push(ch);
        kept += 1;
        if kept >= MAX_EPOCH_CHARS {
            break;
        }
    }
    key
}

const fn is_forbidden_control_or_bidi(ch: char) -> bool {
    let code = ch as u32;
    matches!(code, 0x00..=0x1F | 0x7F | 0x80..=0x9F)
        || matches!(
            ch,
            '\u{061C}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
        )
}

/// Reason why a live remote PTY is currently attached in read-only mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnlyReason {
    /// Another Tessel window or client currently owns the write lease.
    ControlledElsewhere,
    /// This pane explicitly attached as a non-controlling observer.
    ObserverAttach,
}

/// Honest reason why a remote PTY can no longer be resumed in-place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LostReason {
    /// Generic `TerminalEvent::Lost` when the client cannot prove whether the session ended,
    /// mismatched epoch, or the daemon restarted (`client.state()` did not report `Restarted`).
    /// Never claims the host service restarted without proof.
    Unavailable,
    /// Proven `tessel-host` epoch change / `client.state() == Restarted`: previous PTY is gone.
    EpochChanged,
    /// Explicit `NotFound`: the remote PTY was closed or terminated.
    SessionGone,
    /// Transport connection lost and automatic reconnect cannot continue.
    ConnectionLost { detail: Option<String> },
    /// Initial handshake or spawn failed before a remote PTY lease was established.
    StartFailed { detail: String },
}

/// Fixed reason why a local terminal stream stopped or was refused without proving that the
/// remote program exited (`Feed::State::Rejected(_)` / `Feed::State::Closed`).
///
/// Never embeds raw stream bytes or untrusted payload text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamRejectReason {
    /// Snapshot parts arrived out of order, disagreed on screen metadata, exceeded part bounds,
    /// or were interrupted by another event.
    InvalidSnapshot,
    /// Assembled snapshot exceeded `MAX_SNAPSHOT_BYTES` (`16 MiB`).
    OversizedSnapshot,
    /// Terminal stream frame, dimensions, or sequence was locally invalid (not necessarily a snapshot).
    InvalidData,
    /// Local terminal event stream closed permanently (`State::Closed`); automatic reconnect is
    /// not running, and the remote program may still be running on the host.
    Closed,
}

/// Fixed reason why a remote terminal opening attempt (`create` or `attach`) ended without
/// yielding a live attached terminal (`host_opening::Failure`).
///
/// Note: `Failure::EpochChanged` and `Failure::Gone` map to [`HostTerminalPhase::Lost`]
/// (`EpochChanged` / `SessionGone`), and `Failure::Cancelled` closes the pane cleanly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpeningFailureReason {
    /// Local host-terminal capacity limit reached before sending any request (`Failure::Capacity`).
    Capacity,
    /// Local capacity stayed full while holding a pending retry token (`Failure::Deferred`).
    DeferredRetry,
    /// Host client was not connected or `attach` ended ambiguously (`Failure::NotConnected`):
    /// never claims nothing was sent, because an existing remote session may still be running.
    NotConnected,
    /// Local encrypted lease storage failed before sending any request (`Failure::StoreUnavailable`).
    StoreUnavailable,
    /// `create_keyed` lost transport before confirmation (`Failure::Ambiguous`): remote program
    /// may have started; safe deduplicated `RetryOpening` or `BrowseExistingTerminals` is offered.
    Ambiguous,
    /// Deduplication retry window expired (`Failure::Expired`): blind retry is blocked so a
    /// duplicate process is never spawned silently.
    RetryExpired,
    /// Terminal opened on the host, was not adopted by the pane in time, and was detached while
    /// still running (`Failure::Detached`).
    Detached,
    /// Cancellation was requested after the terminal started, and the remote stop RPC could not
    /// confirm termination (`Failure::CancelNotConfirmed`).
    CancelNotConfirmed,
    /// Opening worker stopped unexpectedly (`Failure::Stopped`): remote outcome is unknown,
    /// automatic relaunch is forbidden, and `BrowseExistingTerminals` is offered.
    Stopped,
}

/// Lifecycle phase of a remote `tessel-host` terminal pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostTerminalPhase {
    /// Connecting/provisioning/attaching before a live PTY lease exists.
    Opening { note: Option<String> },
    /// Opening failed before a live PTY stream could be attached (`host_opening::Failure`):
    /// input/resize/DSR/`Enter` are suppressed and only explicit safe recovery actions are shown.
    OpeningFailed { reason: OpeningFailureReason },
    /// Controlling live PTY session (`accepts_input` and `accepts_resize` when unblocked).
    Live,
    /// Observing live PTY output without a write lease (`accepts_input == false`).
    ReadOnly { reason: ReadOnlyReason },
    /// Transport interrupted; automatic reconnect is in progress.
    Reconnecting {
        attempt: u32,
        max_attempts: Option<u32>,
    },
    /// Remote PTY stream was refused locally (`InvalidSnapshot`, `OversizedSnapshot`,
    /// `InvalidData`) or closed permanently (`Closed`): local stream is abandoned without
    /// automatic reattach, previous screen & scrollback remain readable, input/resize/DSR/`Enter`
    /// are suppressed, and the remote program may still be running on the host.
    StreamRejected { reason: StreamRejectReason },
    /// Unrecoverable loss or changed epoch — never resumes old process; explicit `RestartNew` only.
    Lost { reason: LostReason },
    /// Remote shell/command exited normally or via signal.
    Exited {
        exit_code: Option<i32>,
        signal: Option<i32>,
    },
}

/// Logout-survival warning state when `survives_logout != true`.
///
/// Pass `None` to [`HostTerminalInput::persistence_warning`] when `survives_logout == true`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurvivesLogout {
    /// Host explicitly reported `survives_logout == false` (e.g. systemd user lingering disabled).
    No,
    /// Host could not verify whether background processes survive SSH/session logout.
    Unknown,
}

/// Per-epoch logout-persistence warning input (`epoch` is an opaque string, never integer-parsed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PersistenceWarningInput<'a> {
    pub epoch: &'a str,
    pub survives_logout: SurvivesLogout,
}

impl<'a> PersistenceWarningInput<'a> {
    #[must_use]
    pub const fn new(epoch: &'a str, survives_logout: SurvivesLogout) -> Self {
        Self {
            epoch,
            survives_logout,
        }
    }
}

/// Output gap reported after re-attaching or resynchronizing a remote PTY.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkippedOutput {
    /// Exact skipped byte count when explicitly supplied by the transport (`0` is ignored).
    Bytes(u64),
    /// `Snapshot.resync == true`: screen was resynchronized without a byte count in the protocol.
    /// Never invents or displays a synthetic byte number.
    Resynchronized,
}

/// Per-epoch skipped-output / resynchronization notice input (`epoch` is an opaque string).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkippedOutputInput<'a> {
    pub epoch: &'a str,
    pub skipped: SkippedOutput,
}

impl<'a> SkippedOutputInput<'a> {
    #[must_use]
    pub const fn resynchronized(epoch: &'a str) -> Self {
        Self {
            epoch,
            skipped: SkippedOutput::Resynchronized,
        }
    }

    #[must_use]
    pub const fn bytes(epoch: &'a str, bytes: u64) -> Self {
        Self {
            epoch,
            skipped: SkippedOutput::Bytes(bytes),
        }
    }
}

/// Explicit capability flags supplied by the backend/lease owner for the current pane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostTerminalCapabilities {
    /// True only when the pane holds a real capability/lease permitting remote process termination.
    /// UI additionally restricts `Terminate` to [`HostTerminalPhase::Live`],
    /// [`HostTerminalPhase::ReadOnly`], [`HostTerminalPhase::StreamRejected`], and
    /// [`HostTerminalPhase::OpeningFailed`] (`Detached` / `CancelNotConfirmed`) — always after
    /// explicit confirmation.
    pub can_terminate: bool,
    /// True when the pane can detach cleanly (leaving a live PTY running or cancelling attach).
    pub can_detach: bool,
    /// True when a [`HostTerminalPhase::ReadOnly`] pane can request taking write control.
    pub can_take_control: bool,
    /// True when manual retry (`Reconnect`) is available during [`HostTerminalPhase::Reconnecting`].
    pub can_reconnect: bool,
    /// True when starting a brand-new session (`RestartNew`) is allowed in `Lost` or `Exited`.
    pub can_restart: bool,
    /// True when [`HostTerminalPhase::OpeningFailed`] holds a valid retry path (additionally gated
    /// by the UI so `RetryExpired`, `Detached`, `CancelNotConfirmed`, and `Stopped` never retry).
    pub can_retry_opening: bool,
    /// True when the caller can open the existing remote terminals picker for this host.
    pub can_browse_existing: bool,
    /// True when the caller can retry persisting an unsaved terminal lease locally.
    pub can_retry_save_lease: bool,
}

/// Borrowed frame input for [`HostTerminalPanel`].
#[derive(Clone, Copy, Debug)]
pub struct HostTerminalInput<'a> {
    pub id_salt: Id,
    pub host_label: &'a str,
    /// Opaque `tessel-host` daemon epoch string (`None` before handshake completes).
    pub epoch: Option<&'a str>,
    pub phase: &'a HostTerminalPhase,
    pub capabilities: HostTerminalCapabilities,
    /// True only when this specific terminal pane (not merely the OS window or another split pane)
    /// owns keyboard focus. Gates `Enter` -> `RestartNew` in `Exited` and write-attempt flashes in
    /// `ReadOnly` so unfocused split panes never steal keys or react to typing elsewhere.
    pub terminal_focused: bool,
    /// Pure local persistence-status flag (`true` while an in-memory lease for this terminal has
    /// not yet been saved to local encrypted storage). Shown across `Live`, `ReadOnly`,
    /// `Reconnecting`, `StreamRejected`, and `OpeningFailed`; **never** implies write or terminate
    /// authority (authority comes strictly from `capabilities`).
    pub unsaved_lease: bool,
    pub persistence_warning: Option<PersistenceWarningInput<'a>>,
    pub skipped_output: Option<SkippedOutputInput<'a>>,
    pub scrollback_lines: usize,
    pub french: bool,
}

/// Compact `.exit-tag` badge variant for the pane header (`style.css:805-812`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostHeaderTag {
    Opening,
    OpeningFailed,
    ReadOnly,
    Reconnecting,
    StreamRejected,
    StreamClosed,
    Lost,
    Exited,
}

impl HostHeaderTag {
    #[must_use]
    pub const fn label(self, french: bool) -> &'static str {
        match (self, french) {
            (Self::Opening, false) => "Connecting…",
            (Self::Opening, true) => "Connexion…",
            (Self::OpeningFailed, false) => "Open failed",
            (Self::OpeningFailed, true) => "Échec d'ouverture",
            (Self::ReadOnly, false) => "Read-only",
            (Self::ReadOnly, true) => "Lecture seule",
            (Self::Reconnecting, false) => "Reconnecting…",
            (Self::Reconnecting, true) => "Reconnexion…",
            (Self::StreamRejected, false) => "Stream refused",
            (Self::StreamRejected, true) => "Flux refusé",
            (Self::StreamClosed, false) => "Stream closed",
            (Self::StreamClosed, true) => "Flux fermé",
            (Self::Lost, false) => "Disconnected",
            (Self::Lost, true) => "Déconnecté",
            (Self::Exited, false) => "Exited",
            (Self::Exited, true) => "Terminé",
        }
    }
}

/// Measures the outer width of the compact `.exit-tag` header pill so callers can reserve space
/// before truncating the pane title in narrow headers.
#[must_use]
pub fn header_tag_width(ui: &Ui, tag: HostHeaderTag, french: bool) -> f32 {
    let galley = ui.painter().layout_no_wrap(
        tag.label(french).to_owned(),
        FontId::proportional(10.5),
        Color32::WHITE,
    );
    (galley.size().x + 12.0).ceil()
}

/// Renders the compact `.exit-tag` pill inside the pane header (`10.5px`, `padding: 1px 6px`,
/// `border-radius: 4px`, matching `style.css:805-812`), clamping to `ui.available_width()` so a
/// narrow header row never expands its parent `Ui::min_rect` or `terminal_rect`.
pub fn show_header_tag(ui: &mut Ui, tag: HostHeaderTag, french: bool) -> Response {
    let pal = active_palette(ui);
    let dark = ui.visuals().dark_mode;
    let label = tag.label(french);
    let (bg, fg) = match tag {
        HostHeaderTag::Opening => (
            with_alpha(pal.accent, 0.18),
            if dark {
                brighten(pal.accent, 1.15)
            } else {
                pal.accent
            },
        ),
        HostHeaderTag::ReadOnly | HostHeaderTag::Reconnecting | HostHeaderTag::Exited => (
            with_alpha(pal.warn, 0.18),
            if dark { WARN_TEXT_DARK } else { pal.warn },
        ),
        HostHeaderTag::OpeningFailed
        | HostHeaderTag::StreamRejected
        | HostHeaderTag::StreamClosed
        | HostHeaderTag::Lost => (
            with_alpha(pal.danger, 0.22),
            if dark { DANGER_TEXT_DARK } else { pal.danger },
        ),
    };
    let full_galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(10.5), fg);
    let desired_w = (full_galley.size().x + 12.0).ceil();
    let desired_h = (full_galley.size().y + 2.0).ceil().max(16.0);
    let alloc_w = desired_w.min(ui.available_width().floor().max(0.0));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(alloc_w, desired_h), Sense::hover());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
    if alloc_w >= 8.0 {
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        painter.rect_filled(rect, CornerRadius::same(4), bg);
        let galley = if alloc_w < desired_w {
            layout_wrapped_text(ui, label, 10.5, fg, (alloc_w - 8.0).max(1.0), 1)
        } else {
            full_galley
        };
        painter.galley(
            Pos2::new(
                (rect.center().x - galley.size().x * 0.5).max(rect.left() + 4.0),
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            fg,
        );
    }
    resp
}

/// Pure user intent emitted by [`HostTerminalPanel`] for the caller/backend to execute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostTerminalAction {
    /// Trigger an immediate reconnect attempt during [`HostTerminalPhase::Reconnecting`].
    Reconnect,
    /// Request write-lease takeover from [`HostTerminalPhase::ReadOnly`].
    TakeControl,
    /// Explicitly retry a failed terminal opening when [`HostTerminalPhase::OpeningFailed`] allows
    /// safe retry (`Ambiguous`, `DeferredRetry`, `Capacity`, `NotConnected`, `StoreUnavailable`).
    RetryOpening,
    /// Open the existing remote terminals picker for this host so the user can inspect or attach
    /// without risking a duplicate process.
    BrowseExistingTerminals,
    /// Retry persisting an unsaved in-memory terminal lease to local encrypted storage.
    RetrySaveLease,
    /// Explicitly start a brand-new remote PTY after [`HostTerminalPhase::Lost`] or `Exited`.
    RestartNew,
    /// Detach this local pane without killing the remote PTY (or cancel `Opening` attach).
    Detach,
    /// Kill the remote PTY process and close this pane (only when `can_terminate` &
    /// `Live | ReadOnly | StreamRejected | OpeningFailed(Detached | CancelNotConfirmed)`, after
    /// explicit user confirmation).
    Terminate,
    /// Close this local pane after `Lost`, `Exited`, `OpeningFailed`, or when no remote process
    /// lease applies.
    ClosePane,
    /// Scroll the terminal buffer back to live output (`scrollback = 0`).
    BackToLiveOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OverlayButtonStyle {
    Primary,
    Standard,
    SmallPrimary,
    Small,
    Danger,
}

#[derive(Clone)]
struct ButtonSpec {
    id_key: &'static str,
    label: String,
    style: OverlayButtonStyle,
    action: Option<HostTerminalAction>,
    close_confirm: bool,
    open_confirm: bool,
    request_initial_focus: bool,
}

struct BottomOverlaySpec {
    id_key: &'static str,
    text: String,
    danger_border: bool,
    buttons: Vec<ButtonSpec>,
}

struct BottomOverlayOutcome {
    card_rect: Rect,
    emitted_action: Option<HostTerminalAction>,
    close_confirm_clicked: bool,
    open_confirm_clicked: bool,
    focused_any: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BannerTone {
    Accent,
    Warn,
    WarnFlash,
    Neutral,
}

struct TopBannerSpec {
    id_key: &'static str,
    text: String,
    tone: BannerTone,
    buttons: Vec<ButtonSpec>,
    dismiss: Option<DismissSpec>,
}

struct DismissSpec {
    id_key: &'static str,
    label: &'static str,
    kind: DismissKind,
}

enum DismissKind {
    Persistence(String),
    Skipped(String),
}

/// Stateful, pure-UI controller for a single remote terminal pane's overlays and close prompt.
#[derive(Debug, Default)]
pub struct HostTerminalPanel {
    observed_epoch: Option<String>,
    dismissed_persistence_epochs: BTreeSet<String>,
    dismissed_skipped_keys: BTreeSet<String>,
    confirm_close_open: bool,
    confirm_close_focus_pending: bool,
    escape_held: bool,
    readonly_flash_until: Option<f64>,
    last_readonly_notice_at: Option<f64>,
    overlay_rects: Vec<Rect>,
}

impl HostTerminalPanel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// UI routing filter indicating whether keyboard/paste/IME input, terminal mouse tracking,
    /// and `\x1b[6n` DSR auto-replies may be forwarded to the terminal transport.
    ///
    /// **Note:** Pure UI filter, not security authority — `tessel-host` re-checks lease & epoch.
    #[must_use]
    pub const fn accepts_input(&self, phase: &HostTerminalPhase) -> bool {
        matches!(phase, HostTerminalPhase::Live) && !self.confirm_close_open && !self.escape_held
    }

    /// UI routing filter indicating whether local viewport `(rows, cols)` changes may be sent as
    /// remote PTY `resize` frames. Only a controlling [`HostTerminalPhase::Live`] pane resizes the
    /// remote PTY.
    ///
    /// **Note:** Pure UI filter, not security authority — `tessel-host` re-checks lease & epoch.
    #[must_use]
    pub const fn accepts_resize(&self, phase: &HostTerminalPhase) -> bool {
        matches!(phase, HostTerminalPhase::Live)
    }

    /// True while the inline Detach/Terminate close confirmation owns keyboard focus.
    #[must_use]
    pub const fn blocks_terminal_focus(&self) -> bool {
        self.confirm_close_open
    }

    /// Maps a [`HostTerminalPhase`] to its optional pane-header `.exit-tag` badge (`None` in `Live`).
    #[must_use]
    pub const fn header_tag(phase: &HostTerminalPhase) -> Option<HostHeaderTag> {
        match phase {
            HostTerminalPhase::Live => None,
            HostTerminalPhase::Opening { .. } => Some(HostHeaderTag::Opening),
            HostTerminalPhase::OpeningFailed { .. } => Some(HostHeaderTag::OpeningFailed),
            HostTerminalPhase::ReadOnly { .. } => Some(HostHeaderTag::ReadOnly),
            HostTerminalPhase::Reconnecting { .. } => Some(HostHeaderTag::Reconnecting),
            HostTerminalPhase::StreamRejected {
                reason: StreamRejectReason::Closed,
            } => Some(HostHeaderTag::StreamClosed),
            HostTerminalPhase::StreamRejected { .. } => Some(HostHeaderTag::StreamRejected),
            HostTerminalPhase::Lost { .. } => Some(HostHeaderTag::Lost),
            HostTerminalPhase::Exited { .. } => Some(HostHeaderTag::Exited),
        }
    }

    #[must_use]
    pub const fn has_pending_close_choice(&self) -> bool {
        self.confirm_close_open
    }

    pub fn cancel_close_choice(&mut self) {
        self.confirm_close_open = false;
        self.confirm_close_focus_pending = false;
    }

    #[must_use]
    pub fn observed_epoch(&self) -> Option<&str> {
        self.observed_epoch.as_deref()
    }

    /// Screen rectangles of all `.root-note` and `.exit-overlay` cards painted during the most
    /// recent [`Self::show_overlays`] pass.
    #[must_use]
    pub fn overlay_rects(&self) -> &[Rect] {
        &self.overlay_rects
    }

    /// Returns `true` when `pos` lies inside any visible overlay banner or close-choice card.
    #[must_use]
    pub fn pointer_over_overlay(&self, pos: Pos2) -> bool {
        self.overlay_rects.iter().any(|r| r.contains(pos))
    }

    #[must_use]
    pub fn is_readonly_flash_active(&self, now_secs: f64) -> bool {
        self.readonly_flash_until
            .is_some_and(|until| now_secs < until)
    }

    /// Consumes `Escape` to dismiss an open close-choice prompt without leaking autorepeat `\x1b`
    /// frames into the underlying `Live` terminal, while yielding the first `Escape` press to any
    /// open `egui::Popup` menu.
    pub fn handle_escape(&mut self, ctx: &egui::Context) -> bool {
        let down = ctx.input(|i| i.key_down(Key::Escape));
        if !down {
            self.escape_held = false;
        }
        if self.escape_held {
            consume_all_escape_events(ctx);
            return true;
        }
        if !self.confirm_close_open {
            return false;
        }
        if egui::Popup::is_any_open(ctx) {
            self.escape_held = down;
            return false;
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
            self.confirm_close_open = false;
            self.confirm_close_focus_pending = false;
            self.escape_held = down;
            return true;
        }
        false
    }

    /// Pre-terminal keyboard filter:
    /// - Runs [`Self::handle_escape`] so closing a prompt never leaks `\x1b` on held `Escape`
    ///   (an open close-choice prompt retains `Escape` ownership regardless of `terminal_focused`).
    /// - When `input.terminal_focused` is `true`, triggers a 4.0s-throttled read-only highlight
    ///   upon write-intent keys in [`HostTerminalPhase::ReadOnly`] (ignoring copy shortcuts like
    ///   `Ctrl+C`).
    /// - When `input.terminal_focused` is `true`, allows `Enter` to trigger
    ///   [`HostTerminalAction::RestartNew`] **only** in [`HostTerminalPhase::Exited`] (never in
    ///   [`HostTerminalPhase::Lost`] or [`HostTerminalPhase::OpeningFailed`], and never consuming
    ///   `Enter` when another split pane or text field holds focus).
    pub fn intercept_key(
        &mut self,
        ctx: &egui::Context,
        input: &HostTerminalInput<'_>,
    ) -> Option<HostTerminalAction> {
        self.sync_state(input);
        if self.handle_escape(ctx) || self.confirm_close_open || !input.terminal_focused {
            return None;
        }
        match input.phase {
            HostTerminalPhase::Live
            | HostTerminalPhase::Opening { .. }
            | HostTerminalPhase::OpeningFailed { .. }
            | HostTerminalPhase::Reconnecting { .. }
            | HostTerminalPhase::StreamRejected { .. }
            | HostTerminalPhase::Lost { .. } => None,
            HostTerminalPhase::ReadOnly { .. } => {
                let (now, focused, write_attempt) =
                    ctx.input(|i| (i.time, i.focused, has_readonly_write_attempt(&i.events)));
                if focused
                    && write_attempt
                    && self
                        .last_readonly_notice_at
                        .is_none_or(|last| now - last >= READONLY_FLASH_COOLDOWN_SECS)
                {
                    self.last_readonly_notice_at = Some(now);
                    self.readonly_flash_until = Some(now + READONLY_FLASH_DURATION_SECS);
                    ctx.request_repaint_after(Duration::from_millis(1650));
                }
                None
            }
            HostTerminalPhase::Exited { .. } => {
                if !input.capabilities.can_restart || egui::Popup::is_any_open(ctx) {
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
                if enter_pressed {
                    let _ = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                    return Some(HostTerminalAction::RestartNew);
                }
                None
            }
        }
    }

    /// Handles a pane-close request (`×` button or close shortcut):
    /// - In [`HostTerminalPhase::Opening`], never claims a remote process exists and never offers
    ///   `Terminate` — immediately returns `Detach` (if `can_detach`) or `ClosePane`.
    /// - In [`HostTerminalPhase::OpeningFailed`]:
    ///   - For `Detached` or `CancelNotConfirmed` when `can_terminate` authority is held, opens
    ///     the explicit `Leave running / Terminate / Cancel` confirmation (`None`).
    ///   - Otherwise immediately returns `ClosePane`.
    /// - In `Live` or `ReadOnly`:
    ///   - Opens the explicit close-choice card (`None`) when `can_terminate` is available
    ///     (alongside `Detach` if `can_detach`), so the user never kills a remote PTY by accident.
    ///   - Immediately emits `Detach` when `can_detach && !can_terminate`, or `ClosePane` when
    ///     neither capability is held.
    /// - In [`HostTerminalPhase::StreamRejected`]:
    ///   - Opens the explicit `Detach / Terminate / Cancel` confirmation (`None`) when
    ///     `can_terminate` authority is held, because the remote program may still be running.
    ///   - Otherwise immediately returns `Detach` without terminating the remote program.
    /// - In `Reconnecting`: emits `Detach` (if `can_detach`) or `ClosePane`.
    /// - In `Lost` or `Exited`: immediately emits `ClosePane`.
    pub fn request_close(
        &mut self,
        phase: &HostTerminalPhase,
        capabilities: HostTerminalCapabilities,
    ) -> Option<HostTerminalAction> {
        match phase {
            HostTerminalPhase::Opening { .. } | HostTerminalPhase::Reconnecting { .. } => {
                self.cancel_close_choice();
                if capabilities.can_detach {
                    Some(HostTerminalAction::Detach)
                } else {
                    Some(HostTerminalAction::ClosePane)
                }
            }
            HostTerminalPhase::OpeningFailed { .. } => {
                if can_offer_terminate(phase, capabilities) {
                    self.confirm_close_open = true;
                    self.confirm_close_focus_pending = true;
                    None
                } else {
                    self.cancel_close_choice();
                    Some(HostTerminalAction::ClosePane)
                }
            }
            HostTerminalPhase::Live | HostTerminalPhase::ReadOnly { .. } => {
                if can_offer_terminate(phase, capabilities) {
                    self.confirm_close_open = true;
                    self.confirm_close_focus_pending = true;
                    None
                } else if capabilities.can_detach {
                    Some(HostTerminalAction::Detach)
                } else {
                    Some(HostTerminalAction::ClosePane)
                }
            }
            HostTerminalPhase::StreamRejected { .. } => {
                if can_offer_terminate(phase, capabilities) {
                    self.confirm_close_open = true;
                    self.confirm_close_focus_pending = true;
                    None
                } else {
                    self.cancel_close_choice();
                    Some(HostTerminalAction::Detach)
                }
            }
            HostTerminalPhase::Lost { .. } | HostTerminalPhase::Exited { .. } => {
                self.cancel_close_choice();
                Some(HostTerminalAction::ClosePane)
            }
        }
    }

    /// Paints floating `.root-note` top banners and `.exit-overlay` / close-choice bottom cards
    /// inside `terminal_rect` without mutating `ui`'s layout cursor or `min_rect`.
    pub fn show_overlays(
        &mut self,
        ui: &mut Ui,
        terminal_rect: Rect,
        input: &HostTerminalInput<'_>,
    ) -> Option<HostTerminalAction> {
        self.sync_state(input);
        self.overlay_rects.clear();
        if !terminal_rect.is_positive() {
            return None;
        }
        let clip_rect = terminal_rect.intersect(ui.clip_rect());
        if !clip_rect.is_positive() {
            return None;
        }

        let now = ui.input(|i| i.time);
        if let Some(until) = self.readonly_flash_until {
            if now >= until {
                self.readonly_flash_until = None;
            } else {
                ui.ctx().request_repaint_after(Duration::from_secs_f64(
                    (until - now).clamp(0.016, READONLY_FLASH_DURATION_SECS),
                ));
            }
        }

        let pal = active_palette(ui);
        let host = sanitized_host_or_fallback(input.host_label, input.french);
        let flash_active = self.is_readonly_flash_active(now);
        let mut emitted: Option<HostTerminalAction> = None;

        // 1. Top `.root-note` banner stack (`top: 6px; left/right: 14px`, `8px` vertical gap).
        let top_banners = self.build_top_banners(input, &host, flash_active);
        let mut cursor_y = terminal_rect.top() + TOP_BANNER_OFFSET_Y;
        for banner in &top_banners {
            if cursor_y >= terminal_rect.bottom() - 8.0 {
                break;
            }
            let (card_rect, banner_action, dismissed) = paint_top_banner(
                ui,
                clip_rect,
                terminal_rect,
                cursor_y,
                input.id_salt,
                banner,
                &pal,
            );
            self.overlay_rects.push(card_rect);
            cursor_y = card_rect.bottom() + TOP_BANNER_GAP_Y;
            if let Some(kind) = dismissed {
                self.record_dismissal(kind);
            }
            if emitted.is_none() && banner_action.is_some() {
                emitted = banner_action;
            }
        }

        // 2. Bottom `.exit-overlay` or inline Detach/Terminate close confirmation card.
        if let Some(spec) = self.build_bottom_overlay(input, &host) {
            let focus_pending = self.confirm_close_focus_pending;
            let outcome = paint_bottom_overlay(
                ui,
                clip_rect,
                terminal_rect,
                input.id_salt.with(spec.id_key),
                &spec.text,
                spec.danger_border,
                &spec.buttons,
                focus_pending,
                &pal,
            );
            if outcome.focused_any {
                self.confirm_close_focus_pending = false;
            }
            self.overlay_rects.push(outcome.card_rect);
            if outcome.close_confirm_clicked {
                self.cancel_close_choice();
            } else if outcome.open_confirm_clicked {
                self.confirm_close_open = true;
                self.confirm_close_focus_pending = true;
            }
            if emitted.is_none() && outcome.emitted_action.is_some() {
                emitted = outcome.emitted_action;
            }
        }

        emitted
    }

    fn sync_state(&mut self, input: &HostTerminalInput<'_>) {
        let next_epoch = input
            .epoch
            .map(normalize_epoch_key)
            .filter(|ep| !ep.is_empty());
        let epoch_invalidated = self.observed_epoch.is_some() && self.observed_epoch != next_epoch;
        self.observed_epoch = next_epoch;
        if epoch_invalidated {
            self.confirm_close_open = false;
            self.confirm_close_focus_pending = false;
            self.readonly_flash_until = None;
            self.last_readonly_notice_at = None;
        }
        if !can_offer_terminate(input.phase, input.capabilities) {
            self.confirm_close_open = false;
            self.confirm_close_focus_pending = false;
        }
        if !matches!(input.phase, HostTerminalPhase::ReadOnly { .. }) {
            self.readonly_flash_until = None;
        }
    }

    fn record_dismissal(&mut self, kind: DismissKind) {
        match kind {
            DismissKind::Persistence(key) => {
                if self.dismissed_persistence_epochs.len() >= MAX_DISMISSED_KEYS {
                    self.dismissed_persistence_epochs.clear();
                }
                self.dismissed_persistence_epochs.insert(key);
            }
            DismissKind::Skipped(key) => {
                if self.dismissed_skipped_keys.len() >= MAX_DISMISSED_KEYS {
                    self.dismissed_skipped_keys.clear();
                }
                self.dismissed_skipped_keys.insert(key);
            }
        }
    }

    fn build_top_banners(
        &self,
        input: &HostTerminalInput<'_>,
        host: &str,
        flash_active: bool,
    ) -> Vec<TopBannerSpec> {
        let fr = input.french;
        let caps = input.capabilities;
        let mut out = Vec::with_capacity(4);

        match input.phase {
            HostTerminalPhase::Opening { note } => {
                let clean_note = note
                    .as_deref()
                    .map(|n| sanitize_host_text(n, MAX_NOTE_CHARS))
                    .filter(|n| !n.is_empty());
                let text = match (clean_note, fr) {
                    (Some(n), false) => format!("Connecting to {host} — {n}"),
                    (Some(n), true) => format!("Connexion à {host} — {n}"),
                    (None, false) => format!("Connecting to {host}…"),
                    (None, true) => format!("Connexion à {host}…"),
                };
                let cancel_action = if caps.can_detach {
                    HostTerminalAction::Detach
                } else {
                    HostTerminalAction::ClosePane
                };
                out.push(TopBannerSpec {
                    id_key: "opening-banner",
                    text,
                    tone: BannerTone::Accent,
                    buttons: vec![ButtonSpec {
                        id_key: "opening-cancel",
                        label: if fr { "Annuler" } else { "Cancel" }.to_owned(),
                        style: OverlayButtonStyle::Small,
                        action: Some(cancel_action),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    }],
                    dismiss: None,
                });
            }
            HostTerminalPhase::Reconnecting {
                attempt,
                max_attempts,
            } => {
                let current = (*attempt).max(1);
                let text = match (max_attempts.filter(|&m| m >= current), fr) {
                    (Some(max), false) => format!(
                        "Connection to {host} interrupted — reconnecting (attempt {current} of {max})…"
                    ),
                    (Some(max), true) => format!(
                        "Connexion à {host} interrompue — reconnexion (tentative {current} sur {max})…"
                    ),
                    (None, false) => format!(
                        "Connection to {host} interrupted — reconnecting (attempt {current})…"
                    ),
                    (None, true) => format!(
                        "Connexion à {host} interrompue — reconnexion (tentative {current})…"
                    ),
                };
                let mut buttons = Vec::with_capacity(2);
                if caps.can_reconnect {
                    buttons.push(ButtonSpec {
                        id_key: "reconn-now",
                        label: if fr { "Réessayer" } else { "Retry now" }.to_owned(),
                        style: OverlayButtonStyle::SmallPrimary,
                        action: Some(HostTerminalAction::Reconnect),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    });
                }
                if caps.can_detach {
                    buttons.push(ButtonSpec {
                        id_key: "reconn-detach",
                        label: if fr { "Détacher" } else { "Detach" }.to_owned(),
                        style: OverlayButtonStyle::Small,
                        action: Some(HostTerminalAction::Detach),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    });
                } else {
                    buttons.push(ButtonSpec {
                        id_key: "reconn-close",
                        label: if fr { "Fermer le volet" } else { "Close pane" }.to_owned(),
                        style: OverlayButtonStyle::Small,
                        action: Some(HostTerminalAction::ClosePane),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    });
                }
                out.push(TopBannerSpec {
                    id_key: "reconnecting-banner",
                    text,
                    tone: BannerTone::Warn,
                    buttons,
                    dismiss: None,
                });
            }
            HostTerminalPhase::ReadOnly { reason } => {
                let text = match (reason, flash_active, fr) {
                    (ReadOnlyReason::ControlledElsewhere, true, false) => {
                        "Read-only — input blocked while another window controls this session."
                            .to_owned()
                    }
                    (ReadOnlyReason::ControlledElsewhere, true, true) => {
                        "Lecture seule — saisie bloquée pendant qu'une autre fenêtre contrôle cette session."
                            .to_owned()
                    }
                    (ReadOnlyReason::ControlledElsewhere, false, false) => {
                        "Read-only — controlled in another window.".to_owned()
                    }
                    (ReadOnlyReason::ControlledElsewhere, false, true) => {
                        "Lecture seule — contrôlé dans une autre fenêtre.".to_owned()
                    }
                    (ReadOnlyReason::ObserverAttach, true, false) => {
                        "Read-only — input blocked in observer mode.".to_owned()
                    }
                    (ReadOnlyReason::ObserverAttach, true, true) => {
                        "Lecture seule — saisie bloquée en mode observateur.".to_owned()
                    }
                    (ReadOnlyReason::ObserverAttach, false, false) => {
                        "Read-only — attached as observer.".to_owned()
                    }
                    (ReadOnlyReason::ObserverAttach, false, true) => {
                        "Lecture seule — rattaché en observateur.".to_owned()
                    }
                };
                let mut buttons = Vec::new();
                if caps.can_take_control {
                    buttons.push(ButtonSpec {
                        id_key: "readonly-take-control",
                        label: if fr {
                            "Prendre le contrôle"
                        } else {
                            "Take control"
                        }
                        .to_owned(),
                        style: OverlayButtonStyle::SmallPrimary,
                        action: Some(HostTerminalAction::TakeControl),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    });
                }
                out.push(TopBannerSpec {
                    id_key: "readonly-banner",
                    text,
                    tone: if flash_active {
                        BannerTone::WarnFlash
                    } else {
                        BannerTone::Warn
                    },
                    buttons,
                    dismiss: None,
                });
            }
            HostTerminalPhase::Live
            | HostTerminalPhase::OpeningFailed { .. }
            | HostTerminalPhase::StreamRejected { .. }
            | HostTerminalPhase::Lost { .. }
            | HostTerminalPhase::Exited { .. } => {}
        }

        let show_unsaved_lease = input.unsaved_lease
            && !matches!(
                input.phase,
                HostTerminalPhase::Opening { .. }
                    | HostTerminalPhase::Lost { .. }
                    | HostTerminalPhase::Exited { .. }
            );
        if show_unsaved_lease {
            let text = if fr {
                "L'accès à ce terminal n'est pas enregistré sur cet ordinateur — enregistrez-le avant de quitter Tessel pour conserver sa reprise ou son arrêt après redémarrage."
            } else {
                "Terminal access is not saved on this computer — save it before closing Tessel to keep recovery or termination access after a restart."
            }
            .to_owned();
            let mut buttons = Vec::new();
            if caps.can_retry_save_lease {
                buttons.push(ButtonSpec {
                    id_key: "unsaved-lease-retry-save",
                    label: if fr {
                        "Réessayer l'enregistrement"
                    } else {
                        "Retry save"
                    }
                    .to_owned(),
                    style: OverlayButtonStyle::SmallPrimary,
                    action: Some(HostTerminalAction::RetrySaveLease),
                    close_confirm: false,
                    open_confirm: false,
                    request_initial_focus: false,
                });
            }
            out.push(TopBannerSpec {
                id_key: "unsaved-lease-banner",
                text,
                tone: BannerTone::Warn,
                buttons,
                dismiss: None,
            });
        }

        let session_active = matches!(
            input.phase,
            HostTerminalPhase::Live | HostTerminalPhase::ReadOnly { .. }
        );
        if session_active && let Some(pw) = input.persistence_warning {
            let epoch_key = persistence_dismiss_key(pw.epoch, pw.survives_logout);
            if !self.dismissed_persistence_epochs.contains(&epoch_key) {
                let text = match (pw.survives_logout, fr) {
                    (SurvivesLogout::No, false) => format!(
                        "Sessions on {host} end on logout (user lingering is disabled on this host)."
                    ),
                    (SurvivesLogout::No, true) => format!(
                        "Les sessions sur {host} se ferment à la déconnexion (la persistance utilisateur est désactivée sur cet hôte)."
                    ),
                    (SurvivesLogout::Unknown, false) => format!(
                        "Logout persistence could not be verified on {host} — this session may end when you log out."
                    ),
                    (SurvivesLogout::Unknown, true) => format!(
                        "La persistance après déconnexion n'a pas pu être vérifiée sur {host} — cette session peut s'arrêter à la déconnexion."
                    ),
                };
                out.push(TopBannerSpec {
                    id_key: "persistence-banner",
                    text,
                    tone: BannerTone::Warn,
                    buttons: Vec::new(),
                    dismiss: Some(DismissSpec {
                        id_key: "persistence-dismiss",
                        label: if fr {
                            "Fermer l'avis de persistance"
                        } else {
                            "Dismiss persistence notice"
                        },
                        kind: DismissKind::Persistence(epoch_key),
                    }),
                });
            }
        }

        if session_active
            && let Some(so) = input.skipped_output
            && so.skipped != SkippedOutput::Bytes(0)
        {
            let skip_key = skipped_dismiss_key(so.epoch, so.skipped);
            if !self.dismissed_skipped_keys.contains(&skip_key) {
                let text = match (so.skipped, fr) {
                    (SkippedOutput::Resynchronized, false) => {
                        "Output resynchronized — older output while disconnected was not buffered."
                            .to_owned()
                    }
                    (SkippedOutput::Resynchronized, true) => {
                        "Sortie resynchronisée — la sortie plus ancienne produite hors ligne n'a pas été conservée."
                            .to_owned()
                    }
                    (SkippedOutput::Bytes(n), false) => {
                        let formatted = format_byte_count(n, false);
                        format!("Skipped {formatted} of earlier output while reconnecting.")
                    }
                    (SkippedOutput::Bytes(n), true) => {
                        let formatted = format_byte_count(n, true);
                        format!("{formatted} de sortie antérieure ignorés lors de la reconnexion.")
                    }
                };
                let mut buttons = Vec::new();
                if input.scrollback_lines > 0 {
                    buttons.push(ButtonSpec {
                        id_key: "skipped-live-output",
                        label: if fr {
                            "Retour au direct"
                        } else {
                            "Back to live output"
                        }
                        .to_owned(),
                        style: OverlayButtonStyle::Small,
                        action: Some(HostTerminalAction::BackToLiveOutput),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    });
                }
                out.push(TopBannerSpec {
                    id_key: "skipped-output-banner",
                    text,
                    tone: BannerTone::Neutral,
                    buttons,
                    dismiss: Some(DismissSpec {
                        id_key: "skipped-dismiss",
                        label: if fr {
                            "Fermer l'avis de sortie"
                        } else {
                            "Dismiss output notice"
                        },
                        kind: DismissKind::Skipped(skip_key),
                    }),
                });
            }
        }

        out
    }

    fn build_bottom_overlay(
        &self,
        input: &HostTerminalInput<'_>,
        host: &str,
    ) -> Option<BottomOverlaySpec> {
        let fr = input.french;
        let caps = input.capabilities;

        if self.confirm_close_open && can_offer_terminate(input.phase, caps) {
            if matches!(input.phase, HostTerminalPhase::OpeningFailed { .. }) {
                let text = if fr {
                    format!(
                        "Le programme distant sur {host} est peut-être toujours actif — fermer le volet en le laissant tourner, ou terminer son processus ?"
                    )
                } else {
                    format!(
                        "Remote program on {host} may still be running — close pane and leave it running, or terminate its process?"
                    )
                };
                let buttons = vec![
                    ButtonSpec {
                        id_key: "confirm-leave-running",
                        label: if fr {
                            "Laisser tourner"
                        } else {
                            "Leave running"
                        }
                        .to_owned(),
                        style: OverlayButtonStyle::Primary,
                        action: Some(HostTerminalAction::ClosePane),
                        close_confirm: true,
                        open_confirm: false,
                        request_initial_focus: true,
                    },
                    ButtonSpec {
                        id_key: "confirm-terminate",
                        label: if fr {
                            "Terminer le processus"
                        } else {
                            "Terminate process"
                        }
                        .to_owned(),
                        style: OverlayButtonStyle::Danger,
                        action: Some(HostTerminalAction::Terminate),
                        close_confirm: true,
                        open_confirm: false,
                        request_initial_focus: false,
                    },
                    ButtonSpec {
                        id_key: "confirm-cancel",
                        label: if fr { "Annuler" } else { "Cancel" }.to_owned(),
                        style: OverlayButtonStyle::Standard,
                        action: None,
                        close_confirm: true,
                        open_confirm: false,
                        request_initial_focus: false,
                    },
                ];
                return Some(BottomOverlaySpec {
                    id_key: "close-choice-overlay",
                    text,
                    danger_border: true,
                    buttons,
                });
            }
            let is_rejected = matches!(input.phase, HostTerminalPhase::StreamRejected { .. });
            let offer_detach = caps.can_detach || is_rejected;
            let text = match (is_rejected, offer_detach, fr) {
                (true, _, false) => format!(
                    "Remote program on {host} may still be running — detach to leave it running, or terminate its process?"
                ),
                (true, _, true) => format!(
                    "Le programme distant sur {host} est peut-être toujours actif — détacher pour le laisser tourner, ou terminer son processus ?"
                ),
                (false, true, false) => format!(
                    "Remote session on {host} is still running — detach to keep it alive, or terminate its process?"
                ),
                (false, true, true) => format!(
                    "La session distante sur {host} est toujours active — détacher pour la laisser tourner, ou terminer son processus ?"
                ),
                (false, false, false) => format!("Terminate the running remote session on {host}?"),
                (false, false, true) => {
                    format!("Terminer la session distante active sur {host} ?")
                }
            };
            let mut buttons = Vec::with_capacity(3);
            if offer_detach {
                buttons.push(ButtonSpec {
                    id_key: "confirm-detach",
                    label: if fr { "Détacher" } else { "Detach" }.to_owned(),
                    style: OverlayButtonStyle::Primary,
                    action: Some(HostTerminalAction::Detach),
                    close_confirm: true,
                    open_confirm: false,
                    request_initial_focus: true,
                });
            }
            buttons.push(ButtonSpec {
                id_key: "confirm-terminate",
                label: if fr {
                    "Terminer le processus"
                } else {
                    "Terminate process"
                }
                .to_owned(),
                style: OverlayButtonStyle::Danger,
                action: Some(HostTerminalAction::Terminate),
                close_confirm: true,
                open_confirm: false,
                request_initial_focus: false,
            });
            buttons.push(ButtonSpec {
                id_key: "confirm-cancel",
                label: if fr { "Annuler" } else { "Cancel" }.to_owned(),
                style: OverlayButtonStyle::Standard,
                action: None,
                close_confirm: true,
                open_confirm: false,
                request_initial_focus: !offer_detach,
            });
            return Some(BottomOverlaySpec {
                id_key: "close-choice-overlay",
                text,
                danger_border: true,
                buttons,
            });
        }

        match input.phase {
            HostTerminalPhase::OpeningFailed { reason } => {
                let text = format_opening_failed_message(*reason, host, fr);
                let offer_retry = caps.can_retry_opening && opening_failure_allows_retry(*reason);
                let mut buttons = Vec::with_capacity(4);
                if offer_retry {
                    buttons.push(ButtonSpec {
                        id_key: "opening-failed-retry",
                        label: if fr { "Réessayer" } else { "Retry" }.to_owned(),
                        style: OverlayButtonStyle::Primary,
                        action: Some(HostTerminalAction::RetryOpening),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    });
                }
                if caps.can_browse_existing {
                    let browse_style = if offer_retry {
                        OverlayButtonStyle::Standard
                    } else {
                        OverlayButtonStyle::Primary
                    };
                    buttons.push(ButtonSpec {
                        id_key: "opening-failed-browse-existing",
                        label: if fr {
                            "Terminaux existants…"
                        } else {
                            "Existing terminals…"
                        }
                        .to_owned(),
                        style: browse_style,
                        action: Some(HostTerminalAction::BrowseExistingTerminals),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    });
                }
                if opening_failure_may_run_process(*reason) && caps.can_terminate {
                    buttons.push(ButtonSpec {
                        id_key: "opening-failed-terminate",
                        label: if fr {
                            "Terminer le processus"
                        } else {
                            "Terminate process"
                        }
                        .to_owned(),
                        style: OverlayButtonStyle::Danger,
                        action: None,
                        close_confirm: false,
                        open_confirm: true,
                        request_initial_focus: false,
                    });
                }
                buttons.push(ButtonSpec {
                    id_key: "opening-failed-close",
                    label: if fr { "Fermer le volet" } else { "Close pane" }.to_owned(),
                    style: OverlayButtonStyle::Standard,
                    action: Some(HostTerminalAction::ClosePane),
                    close_confirm: false,
                    open_confirm: false,
                    request_initial_focus: false,
                });
                Some(BottomOverlaySpec {
                    id_key: "opening-failed-overlay",
                    text,
                    danger_border: true,
                    buttons,
                })
            }
            HostTerminalPhase::StreamRejected { reason } => {
                let text = format_stream_rejected_message(*reason, host, fr);
                let mut buttons = vec![ButtonSpec {
                    id_key: "stream-rejected-detach",
                    label: if fr { "Détacher" } else { "Detach" }.to_owned(),
                    style: OverlayButtonStyle::Primary,
                    action: Some(HostTerminalAction::Detach),
                    close_confirm: false,
                    open_confirm: false,
                    request_initial_focus: false,
                }];
                if caps.can_terminate {
                    buttons.push(ButtonSpec {
                        id_key: "stream-rejected-terminate",
                        label: if fr {
                            "Terminer le processus"
                        } else {
                            "Terminate process"
                        }
                        .to_owned(),
                        style: OverlayButtonStyle::Danger,
                        action: None,
                        close_confirm: false,
                        open_confirm: true,
                        request_initial_focus: false,
                    });
                }
                Some(BottomOverlaySpec {
                    id_key: "stream-rejected-overlay",
                    text,
                    danger_border: true,
                    buttons,
                })
            }
            HostTerminalPhase::Lost { reason } => {
                let text = format_lost_message(reason, host, fr);
                let mut buttons = Vec::with_capacity(2);
                if caps.can_restart {
                    buttons.push(ButtonSpec {
                        id_key: "lost-restart-new",
                        label: if fr {
                            "Nouvelle session"
                        } else {
                            "Start new session"
                        }
                        .to_owned(),
                        style: OverlayButtonStyle::Primary,
                        action: Some(HostTerminalAction::RestartNew),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    });
                }
                buttons.push(ButtonSpec {
                    id_key: "lost-close-pane",
                    label: if fr { "Fermer le volet" } else { "Close pane" }.to_owned(),
                    style: OverlayButtonStyle::Standard,
                    action: Some(HostTerminalAction::ClosePane),
                    close_confirm: false,
                    open_confirm: false,
                    request_initial_focus: false,
                });
                Some(BottomOverlaySpec {
                    id_key: "lost-overlay",
                    text,
                    danger_border: true,
                    buttons,
                })
            }
            HostTerminalPhase::Exited { exit_code, signal } => {
                let text = format_exited_message(*exit_code, *signal, caps.can_restart, fr);
                let mut buttons = Vec::with_capacity(2);
                if caps.can_restart {
                    buttons.push(ButtonSpec {
                        id_key: "exited-restart-new",
                        label: if fr {
                            "Nouvelle session"
                        } else {
                            "Start new session"
                        }
                        .to_owned(),
                        style: OverlayButtonStyle::Primary,
                        action: Some(HostTerminalAction::RestartNew),
                        close_confirm: false,
                        open_confirm: false,
                        request_initial_focus: false,
                    });
                }
                buttons.push(ButtonSpec {
                    id_key: "exited-close-pane",
                    label: if fr { "Fermer le volet" } else { "Close pane" }.to_owned(),
                    style: OverlayButtonStyle::Standard,
                    action: Some(HostTerminalAction::ClosePane),
                    close_confirm: false,
                    open_confirm: false,
                    request_initial_focus: false,
                });
                Some(BottomOverlaySpec {
                    id_key: "exited-overlay",
                    text,
                    danger_border: false,
                    buttons,
                })
            }
            HostTerminalPhase::Opening { .. }
            | HostTerminalPhase::Live
            | HostTerminalPhase::ReadOnly { .. }
            | HostTerminalPhase::Reconnecting { .. } => None,
        }
    }
}

const fn opening_failure_allows_retry(reason: OpeningFailureReason) -> bool {
    matches!(
        reason,
        OpeningFailureReason::Capacity
            | OpeningFailureReason::DeferredRetry
            | OpeningFailureReason::NotConnected
            | OpeningFailureReason::StoreUnavailable
            | OpeningFailureReason::Ambiguous
    )
}

const fn opening_failure_may_run_process(reason: OpeningFailureReason) -> bool {
    matches!(
        reason,
        OpeningFailureReason::Detached | OpeningFailureReason::CancelNotConfirmed
    )
}

const fn can_offer_terminate(phase: &HostTerminalPhase, caps: HostTerminalCapabilities) -> bool {
    if !caps.can_terminate {
        return false;
    }
    match phase {
        HostTerminalPhase::Live
        | HostTerminalPhase::ReadOnly { .. }
        | HostTerminalPhase::StreamRejected { .. } => true,
        HostTerminalPhase::OpeningFailed { reason } => opening_failure_may_run_process(*reason),
        HostTerminalPhase::Opening { .. }
        | HostTerminalPhase::Reconnecting { .. }
        | HostTerminalPhase::Lost { .. }
        | HostTerminalPhase::Exited { .. } => false,
    }
}

fn format_opening_failed_message(reason: OpeningFailureReason, host: &str, fr: bool) -> String {
    match (reason, fr) {
        (OpeningFailureReason::Capacity, false) => {
            "Too many remote terminals are open locally — close another terminal pane, then retry."
                .to_owned()
        }
        (OpeningFailureReason::Capacity, true) => {
            "Trop de terminaux distants sont ouverts localement — fermez un autre volet de terminal puis réessayez."
                .to_owned()
        }
        (OpeningFailureReason::DeferredRetry, false) => {
            "Too many remote terminals are open locally — close another pane to retry this pending terminal safely."
                .to_owned()
        }
        (OpeningFailureReason::DeferredRetry, true) => {
            "Trop de terminaux distants sont ouverts localement — fermez un autre volet pour réessayer ce terminal en attente sans doublon."
                .to_owned()
        }
        (OpeningFailureReason::NotConnected, false) => format!(
            "Could not reach {host} to open this terminal — an existing remote session may still be running."
        ),
        (OpeningFailureReason::NotConnected, true) => format!(
            "Impossible de joindre {host} pour ouvrir ce terminal — une session distante existante est peut-être toujours active."
        ),
        (OpeningFailureReason::StoreUnavailable, false) => {
            "Local encrypted terminal storage is unavailable — terminal request was not sent."
                .to_owned()
        }
        (OpeningFailureReason::StoreUnavailable, true) => {
            "Le stockage chiffré local des terminaux est indisponible — la demande n'a pas été envoyée."
                .to_owned()
        }
        (OpeningFailureReason::Ambiguous, false) => format!(
            "Opening terminal on {host} was interrupted before confirmation — the remote program may have started; retry safely or check existing terminals."
        ),
        (OpeningFailureReason::Ambiguous, true) => format!(
            "L'ouverture du terminal sur {host} a été interrompue avant confirmation — le programme distant a peut-être démarré ; réessayez sans doublon ou vérifiez les terminaux existants."
        ),
        (OpeningFailureReason::RetryExpired, false) => format!(
            "Safe retry window expired for the unconfirmed terminal on {host} — check existing terminals to avoid starting a duplicate."
        ),
        (OpeningFailureReason::RetryExpired, true) => format!(
            "Le délai de réessai sûr a expiré pour le terminal non confirmé sur {host} — vérifiez les terminaux existants pour éviter un doublon."
        ),
        (OpeningFailureReason::Detached, false) => format!(
            "Terminal on {host} was not adopted in time and was detached — the remote program may still be running."
        ),
        (OpeningFailureReason::Detached, true) => format!(
            "Le terminal sur {host} n'a pas été rattaché à temps et a été détaché — le programme distant est peut-être toujours actif."
        ),
        (OpeningFailureReason::CancelNotConfirmed, false) => format!(
            "Stopping the new terminal on {host} could not be confirmed — the remote program may still be running."
        ),
        (OpeningFailureReason::CancelNotConfirmed, true) => format!(
            "L'arrêt du nouveau terminal sur {host} n'a pas pu être confirmé — le programme distant est peut-être toujours actif."
        ),
        (OpeningFailureReason::Stopped, false) => format!(
            "Terminal opening worker stopped unexpectedly for {host} — outcome is unknown; check existing terminals before opening another."
        ),
        (OpeningFailureReason::Stopped, true) => format!(
            "L'ouverture du terminal pour {host} s'est interrompue de façon inattendue — résultat inconnu ; vérifiez les terminaux existants avant d'en ouvrir un autre."
        ),
    }
}

fn format_stream_rejected_message(reason: StreamRejectReason, host: &str, french: bool) -> String {
    match (reason, french) {
        (StreamRejectReason::InvalidSnapshot, false) => format!(
            "Terminal stream from {host} was refused (invalid screen snapshot) — the remote program may still be running."
        ),
        (StreamRejectReason::InvalidSnapshot, true) => format!(
            "Le flux du terminal sur {host} a été refusé (instantané d'écran invalide) — le programme distant est peut-être toujours actif."
        ),
        (StreamRejectReason::OversizedSnapshot, false) => format!(
            "Terminal stream from {host} was refused (screen snapshot too large) — the remote program may still be running."
        ),
        (StreamRejectReason::OversizedSnapshot, true) => format!(
            "Le flux du terminal sur {host} a été refusé (instantané d'écran trop volumineux) — le programme distant est peut-être toujours actif."
        ),
        (StreamRejectReason::InvalidData, false) => format!(
            "Terminal stream from {host} was refused (invalid terminal data) — the remote program may still be running."
        ),
        (StreamRejectReason::InvalidData, true) => format!(
            "Le flux du terminal sur {host} a été refusé (données du terminal invalides) — le programme distant est peut-être toujours actif."
        ),
        (StreamRejectReason::Closed, false) => format!(
            "Terminal stream from {host} closed permanently — the remote program may still be running."
        ),
        (StreamRejectReason::Closed, true) => format!(
            "Le flux du terminal sur {host} s'est fermé définitivement — le programme distant est peut-être toujours actif."
        ),
    }
}

fn sanitized_host_or_fallback(raw: &str, french: bool) -> String {
    let clean = sanitize_host_text(raw, MAX_HOST_LABEL_CHARS);
    if clean.is_empty() {
        if french {
            "l'hôte distant".to_owned()
        } else {
            "remote host".to_owned()
        }
    } else {
        clean
    }
}

fn persistence_dismiss_key(epoch: &str, survives: SurvivesLogout) -> String {
    let ep = normalize_epoch_key(epoch);
    let tag = match survives {
        SurvivesLogout::No => "no",
        SurvivesLogout::Unknown => "unknown",
    };
    format!("{tag}:{ep}")
}

fn skipped_dismiss_key(epoch: &str, skipped: SkippedOutput) -> String {
    let ep = normalize_epoch_key(epoch);
    match skipped {
        SkippedOutput::Resynchronized => format!("resync:{ep}"),
        SkippedOutput::Bytes(n) => format!("bytes:{n}:{ep}"),
    }
}

fn format_byte_count(bytes: u64, french: bool) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    if bytes < 1024 {
        return if french {
            format!("{bytes} o")
        } else {
            format!("{bytes} B")
        };
    }
    let b = bytes as f64;
    let (val, unit_en, unit_fr) = if b < MB {
        (b / KB, "KB", "Ko")
    } else if b < GB {
        (b / MB, "MB", "Mo")
    } else {
        (b / GB, "GB", "Go")
    };
    let formatted = format!("{val:.1}");
    if french {
        format!("{} {unit_fr}", formatted.replace('.', ","))
    } else {
        format!("{formatted} {unit_en}")
    }
}

fn format_lost_message(reason: &LostReason, host: &str, french: bool) -> String {
    match reason {
        LostReason::Unavailable => {
            if french {
                format!(
                    "La session distante sur {host} n'est plus disponible — démarrez une nouvelle session pour continuer."
                )
            } else {
                format!(
                    "Remote session on {host} is no longer available — start a new session to continue."
                )
            }
        }
        LostReason::EpochChanged => {
            if french {
                format!(
                    "Le service hôte distant a redémarré sur {host} — la session précédente n'existe plus."
                )
            } else {
                format!("Remote host service restarted on {host} — previous session is gone.")
            }
        }
        LostReason::SessionGone => {
            if french {
                format!("La session distante sur {host} a été fermée.")
            } else {
                format!("Remote session on {host} was closed.")
            }
        }
        LostReason::ConnectionLost { detail } => {
            let clean = detail
                .as_deref()
                .map(|d| sanitize_host_text(d, MAX_NOTE_CHARS))
                .filter(|d| !d.is_empty());
            match (clean, french) {
                (Some(d), false) => format!("Connection to {host} lost — {d}"),
                (Some(d), true) => format!("Connexion à {host} perdue — {d}"),
                (None, false) => format!("Connection to {host} lost."),
                (None, true) => format!("Connexion à {host} perdue."),
            }
        }
        LostReason::StartFailed { detail } => {
            let clean = sanitize_host_text(detail, MAX_NOTE_CHARS);
            match (clean.is_empty(), french) {
                (false, false) => format!("Could not start remote session on {host} — {clean}"),
                (false, true) => {
                    format!("Impossible de démarrer la session distante sur {host} — {clean}")
                }
                (true, false) => format!("Could not start remote session on {host}."),
                (true, true) => {
                    format!("Impossible de démarrer la session distante sur {host}.")
                }
            }
        }
    }
}

fn format_exited_message(
    exit_code: Option<i32>,
    signal: Option<i32>,
    can_restart: bool,
    french: bool,
) -> String {
    let suffix = match (can_restart, french) {
        (true, false) => " — press Enter or start a new session",
        (true, true) => " — appuyez sur Entrée ou démarrez une nouvelle session",
        (false, _) => "",
    };
    match (exit_code, signal, french) {
        (Some(code), _, false) => format!("Session ended (code {code}){suffix}"),
        (Some(code), _, true) => format!("Session terminée (code {code}){suffix}"),
        (None, Some(sig), false) => format!("Session ended (signal {sig}){suffix}"),
        (None, Some(sig), true) => format!("Session terminée (signal {sig}){suffix}"),
        (None, None, false) => format!("Session ended{suffix}"),
        (None, None, true) => format!("Session terminée{suffix}"),
    }
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

fn has_readonly_write_attempt(events: &[Event]) -> bool {
    events.iter().any(|ev| match ev {
        Event::Text(s) | Event::Paste(s) => !s.is_empty(),
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            if (modifiers.ctrl || modifiers.command)
                && matches!(key, Key::C | Key::Insert | Key::F | Key::A)
            {
                return false;
            }
            if modifiers.alt && !modifiers.ctrl {
                return false;
            }
            matches!(
                key,
                Key::Enter
                    | Key::Backspace
                    | Key::Delete
                    | Key::Space
                    | Key::ArrowUp
                    | Key::ArrowDown
                    | Key::ArrowLeft
                    | Key::ArrowRight
                    | Key::Home
                    | Key::End
                    | Key::PageUp
                    | Key::PageDown
                    | Key::A
                    | Key::B
                    | Key::C
                    | Key::D
                    | Key::E
                    | Key::F
                    | Key::G
                    | Key::H
                    | Key::I
                    | Key::J
                    | Key::K
                    | Key::L
                    | Key::M
                    | Key::N
                    | Key::O
                    | Key::P
                    | Key::Q
                    | Key::R
                    | Key::S
                    | Key::T
                    | Key::U
                    | Key::V
                    | Key::W
                    | Key::X
                    | Key::Y
                    | Key::Z
                    | Key::Num0
                    | Key::Num1
                    | Key::Num2
                    | Key::Num3
                    | Key::Num4
                    | Key::Num5
                    | Key::Num6
                    | Key::Num7
                    | Key::Num8
                    | Key::Num9
            )
        }
        _ => false,
    })
}

fn layout_wrapped_text(
    ui: &Ui,
    text: &str,
    font_px: f32,
    color: Color32,
    max_width: f32,
    max_rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            font_id: FontId::proportional(font_px),
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width: max_width.max(12.0),
        max_rows,
        break_anywhere: max_width < 96.0,
        overflow_character: Some('…'),
    };
    ui.fonts_mut(|f| f.layout_job(job))
}

fn measure_button_width(ui: &Ui, label: &str, is_small: bool) -> f32 {
    let font_px = if is_small { 11.5 } else { 12.0 };
    let pad_x = if is_small { 18.0 } else { 22.0 };
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(font_px),
        Color32::WHITE,
    );
    (galley.size().x + pad_x).ceil().max(48.0)
}

fn compute_wrapped_button_rows(
    widths: &[f32],
    max_row_w: f32,
    gap_x: f32,
) -> Vec<(usize, usize, f32)> {
    let mut rows = Vec::new();
    if widths.is_empty() {
        return rows;
    }
    let mut start = 0;
    let mut row_w = 0.0_f32;
    for (idx, &raw_w) in widths.iter().enumerate() {
        let w = raw_w.min(max_row_w);
        let next_w = if idx == start { w } else { row_w + gap_x + w };
        if idx > start && next_w > max_row_w {
            rows.push((start, idx, row_w));
            start = idx;
            row_w = w;
        } else {
            row_w = next_w;
        }
    }
    rows.push((start, widths.len(), row_w));
    rows
}

#[allow(clippy::too_many_arguments)]
fn paint_top_banner(
    ui: &Ui,
    clip_rect: Rect,
    terminal_rect: Rect,
    top_y: f32,
    id_salt: Id,
    spec: &TopBannerSpec,
    pal: &UiPalette,
) -> (Rect, Option<HostTerminalAction>, Option<DismissKind>) {
    let side_pad = if terminal_rect.width() < 180.0 {
        4.0
    } else {
        14.0
    };
    let card_left = terminal_rect.left() + side_pad;
    let card_right = (terminal_rect.right() - side_pad)
        .max(card_left + 32.0)
        .min(terminal_rect.right());
    let card_w = (card_right - card_left).max(16.0);
    let pad_x = 10.0_f32.min((card_w * 0.1).max(4.0));
    let pad_y = 7.0_f32;
    let inner_w = (card_w - pad_x * 2.0).max(16.0);

    let btn_widths: Vec<f32> = spec
        .buttons
        .iter()
        .map(|b| measure_button_width(ui, &b.label, true).min(inner_w))
        .collect();
    let buttons_inline_w: f32 = if btn_widths.is_empty() {
        0.0
    } else {
        btn_widths.iter().sum::<f32>() + 6.0 * ((btn_widths.len() - 1) as f32)
    };
    let dismiss_slot_w = if spec.dismiss.is_some() {
        DISMISS_BTN_SIDE + 6.0
    } else {
        0.0
    };
    let total_controls_w = if btn_widths.is_empty() {
        dismiss_slot_w
    } else {
        buttons_inline_w + 10.0 + dismiss_slot_w
    };

    let inline_controls = btn_widths.is_empty() || (inner_w - total_controls_w >= 150.0);
    let text_color = if matches!(spec.tone, BannerTone::Warn | BannerTone::WarnFlash)
        && ui.visuals().dark_mode
    {
        WARN_TEXT_DARK
    } else {
        pal.text
    };

    let text_max_w = if inline_controls {
        (inner_w - total_controls_w).max(16.0)
    } else {
        (inner_w - dismiss_slot_w).max(16.0)
    };
    let text_galley = layout_wrapped_text(ui, &spec.text, 12.0, text_color, text_max_w, 4);

    let btn_rows = if inline_controls {
        Vec::new()
    } else {
        compute_wrapped_button_rows(&btn_widths, inner_w, 6.0)
    };
    let stacked_btns_h = if btn_rows.is_empty() {
        0.0
    } else {
        (btn_rows.len() as f32) * SMALL_BTN_HEIGHT + ((btn_rows.len() - 1) as f32) * 6.0
    };

    let min_row_h = if inline_controls && !btn_widths.is_empty() {
        SMALL_BTN_HEIGHT
    } else if spec.dismiss.is_some() {
        DISMISS_BTN_SIDE
    } else {
        18.0
    };
    let top_row_h = text_galley.size().y.max(min_row_h);
    let card_h = if inline_controls || stacked_btns_h == 0.0 {
        pad_y * 2.0 + top_row_h
    } else {
        pad_y * 2.0 + top_row_h + 6.0 + stacked_btns_h
    };

    let card_rect = Rect::from_min_size(Pos2::new(card_left, top_y), Vec2::new(card_w, card_h));
    let banner_id = id_salt.with(spec.id_key);
    let _ = ui.interact(card_rect, banner_id.with("bg"), Sense::click_and_drag());

    let (fill, stroke) = match spec.tone {
        BannerTone::Accent => (
            pal.surface_2,
            Stroke::new(1.0_f32, with_alpha(pal.accent, 0.55)),
        ),
        BannerTone::Warn => (
            pal.surface_2,
            Stroke::new(1.0_f32, with_alpha(pal.warn, 0.60)),
        ),
        BannerTone::WarnFlash => (
            mix_color(pal.warn, pal.surface_2, 0.18),
            Stroke::new(1.5_f32, pal.warn),
        ),
        BannerTone::Neutral => (pal.surface_2, Stroke::new(1.0_f32, pal.border_strong)),
    };

    let painter = ui.painter().with_clip_rect(clip_rect);
    painter.rect(
        card_rect,
        CornerRadius::same(8),
        fill,
        stroke,
        StrokeKind::Inside,
    );

    let text_y = if inline_controls {
        card_rect.top() + pad_y + (top_row_h - text_galley.size().y) * 0.5
    } else {
        card_rect.top() + pad_y
    };
    painter.galley(
        Pos2::new(card_rect.left() + pad_x, text_y),
        text_galley,
        text_color,
    );

    let mut dismissed_kind: Option<DismissKind> = None;
    let right_limit = if let Some(dismiss) = &spec.dismiss {
        let x = (card_rect.right() - pad_x - DISMISS_BTN_SIDE).max(card_rect.left());
        let y = if inline_controls {
            card_rect.top() + pad_y + (top_row_h - DISMISS_BTN_SIDE) * 0.5
        } else {
            card_rect.top() + pad_y - 1.0
        };
        let d_rect = Rect::from_min_size(Pos2::new(x, y), Vec2::splat(DISMISS_BTN_SIDE));
        let d_resp = paint_dismiss_button(
            ui,
            &painter,
            d_rect,
            banner_id.with(dismiss.id_key),
            dismiss.label,
            pal,
        );
        if d_resp.clicked() {
            dismissed_kind = Some(match &dismiss.kind {
                DismissKind::Persistence(k) => DismissKind::Persistence(k.clone()),
                DismissKind::Skipped(k) => DismissKind::Skipped(k.clone()),
            });
        }
        x - 6.0
    } else {
        card_rect.right() - pad_x
    };

    let mut clicked_action = None;
    if inline_controls && !spec.buttons.is_empty() {
        let mut bx = right_limit - buttons_inline_w;
        let by = card_rect.top() + pad_y + (top_row_h - SMALL_BTN_HEIGHT) * 0.5;
        for (btn, &bw) in spec.buttons.iter().zip(&btn_widths) {
            let b_rect = Rect::from_min_size(Pos2::new(bx, by), Vec2::new(bw, SMALL_BTN_HEIGHT));
            let resp = paint_overlay_button(
                ui,
                &painter,
                b_rect,
                banner_id.with(btn.id_key),
                &btn.label,
                btn.style,
                pal,
            );
            if resp.clicked() && clicked_action.is_none() {
                clicked_action = btn.action;
            }
            bx += bw + 6.0;
        }
    } else if !btn_rows.is_empty() {
        let mut row_y = card_rect.top() + pad_y + top_row_h + 6.0;
        for &(start, end, _) in &btn_rows {
            let mut bx = card_rect.left() + pad_x;
            for (idx, width) in btn_widths.iter().enumerate().take(end).skip(start) {
                let btn = &spec.buttons[idx];
                let bw = width.min(inner_w);
                let b_rect =
                    Rect::from_min_size(Pos2::new(bx, row_y), Vec2::new(bw, SMALL_BTN_HEIGHT));
                let resp = paint_overlay_button(
                    ui,
                    &painter,
                    b_rect,
                    banner_id.with(btn.id_key),
                    &btn.label,
                    btn.style,
                    pal,
                );
                if resp.clicked() && clicked_action.is_none() {
                    clicked_action = btn.action;
                }
                bx += bw + 6.0;
            }
            row_y += SMALL_BTN_HEIGHT + 6.0;
        }
    }

    (card_rect, clicked_action, dismissed_kind)
}

#[allow(clippy::too_many_arguments)]
fn paint_bottom_overlay(
    ui: &Ui,
    clip_rect: Rect,
    terminal_rect: Rect,
    overlay_id: Id,
    text: &str,
    danger_border: bool,
    buttons: &[ButtonSpec],
    focus_pending: bool,
    pal: &UiPalette,
) -> BottomOverlayOutcome {
    let side_pad = if terminal_rect.width() < 180.0 {
        4.0
    } else {
        12.0
    };
    let max_card_w = (terminal_rect.width() - side_pad * 2.0)
        .clamp(32.0, 680.0)
        .min(terminal_rect.width());
    let pad_x = 14.0_f32.min((max_card_w * 0.1).max(4.0));
    let pad_y = 9.0_f32;
    let inner_max_w = (max_card_w - pad_x * 2.0).max(16.0);

    let single_galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::proportional(12.5), pal.text);
    let btn_widths: Vec<f32> = buttons
        .iter()
        .map(|b| measure_button_width(ui, &b.label, false).min(inner_max_w))
        .collect();
    let single_btns_w: f32 = if btn_widths.is_empty() {
        0.0
    } else {
        btn_widths.iter().sum::<f32>() + 8.0 * ((btn_widths.len() - 1) as f32)
    };

    let single_row_total_inner = single_galley.size().x
        + if btn_widths.is_empty() {
            0.0
        } else {
            12.0 + single_btns_w
        };
    let single_row = single_row_total_inner <= inner_max_w;

    let (card_w, card_h, wrapped_galley, btn_rows) = if single_row {
        let w = (single_row_total_inner + pad_x * 2.0)
            .ceil()
            .clamp(40.0_f32.min(max_card_w), max_card_w);
        let h = pad_y * 2.0 + single_galley.size().y.max(EXIT_BTN_HEIGHT);
        (w, h, single_galley, Vec::new())
    } else {
        let galley = layout_wrapped_text(ui, text, 12.5, pal.text, inner_max_w, 5);
        let rows = compute_wrapped_button_rows(&btn_widths, inner_max_w, 8.0);
        let max_row_w = rows.iter().map(|r| r.2).fold(0.0_f32, f32::max);
        let btns_h = if rows.is_empty() {
            0.0
        } else {
            (rows.len() as f32) * EXIT_BTN_HEIGHT + ((rows.len() - 1) as f32) * 6.0
        };
        let used_inner_w = galley.size().x.max(max_row_w).min(inner_max_w);
        let w = (used_inner_w + pad_x * 2.0)
            .ceil()
            .clamp(40.0_f32.min(max_card_w), max_card_w);
        let h = pad_y * 2.0 + galley.size().y + if btns_h > 0.0 { 8.0 + btns_h } else { 0.0 };
        (w, h, galley, rows)
    };

    let card_left = (terminal_rect.center().x - card_w * 0.5)
        .max(terminal_rect.left())
        .min((terminal_rect.right() - card_w).max(terminal_rect.left()));
    let desired_bottom =
        (terminal_rect.bottom() - BOTTOM_OVERLAY_OFFSET_Y).min(terminal_rect.bottom());
    let card_top = (desired_bottom - card_h).max(terminal_rect.top());
    let card_rect = Rect::from_min_size(Pos2::new(card_left, card_top), Vec2::new(card_w, card_h));

    let _ = ui.interact(card_rect, overlay_id.with("bg"), Sense::click_and_drag());

    let border_col = if danger_border {
        with_alpha(pal.danger, 0.55)
    } else {
        pal.border_strong
    };
    let painter = ui.painter().with_clip_rect(clip_rect);
    painter.rect(
        card_rect,
        CornerRadius::same(10),
        pal.surface_2,
        Stroke::new(1.0_f32, border_col),
        StrokeKind::Inside,
    );

    let mut emitted_action = None;
    let mut close_confirm_clicked = false;
    let mut open_confirm_clicked = false;
    let mut focused_any = false;

    if single_row {
        let ty = card_rect.center().y - wrapped_galley.size().y * 0.5;
        painter.galley(
            Pos2::new(card_rect.left() + pad_x, ty),
            wrapped_galley,
            pal.text,
        );
        let mut bx = card_rect.right() - pad_x - single_btns_w;
        let by = card_rect.center().y - EXIT_BTN_HEIGHT * 0.5;
        for (btn, &bw) in buttons.iter().zip(&btn_widths) {
            let b_rect = Rect::from_min_size(Pos2::new(bx, by), Vec2::new(bw, EXIT_BTN_HEIGHT));
            let resp = paint_overlay_button(
                ui,
                &painter,
                b_rect,
                overlay_id.with(btn.id_key),
                &btn.label,
                btn.style,
                pal,
            );
            if focus_pending && btn.request_initial_focus {
                resp.request_focus();
                focused_any = true;
            }
            if resp.clicked() {
                if btn.close_confirm {
                    close_confirm_clicked = true;
                }
                if btn.open_confirm {
                    open_confirm_clicked = true;
                }
                if emitted_action.is_none() {
                    emitted_action = btn.action;
                }
            }
            bx += bw + 8.0;
        }
    } else {
        let tx =
            (card_rect.center().x - wrapped_galley.size().x * 0.5).max(card_rect.left() + pad_x);
        let ty = card_rect.top() + pad_y;
        let text_bottom = ty + wrapped_galley.size().y;
        painter.galley(Pos2::new(tx, ty), wrapped_galley, pal.text);

        let mut row_y = text_bottom + 8.0;
        for &(start, end, row_w) in &btn_rows {
            let mut bx = (card_rect.center().x - row_w * 0.5).max(card_rect.left() + pad_x);
            for idx in start..end {
                let btn = &buttons[idx];
                let bw = btn_widths[idx].min(inner_max_w);
                let b_rect =
                    Rect::from_min_size(Pos2::new(bx, row_y), Vec2::new(bw, EXIT_BTN_HEIGHT));
                let resp = paint_overlay_button(
                    ui,
                    &painter,
                    b_rect,
                    overlay_id.with(btn.id_key),
                    &btn.label,
                    btn.style,
                    pal,
                );
                if focus_pending && btn.request_initial_focus {
                    resp.request_focus();
                    focused_any = true;
                }
                if resp.clicked() {
                    if btn.close_confirm {
                        close_confirm_clicked = true;
                    }
                    if btn.open_confirm {
                        open_confirm_clicked = true;
                    }
                    if emitted_action.is_none() {
                        emitted_action = btn.action;
                    }
                }
                bx += bw + 8.0;
            }
            row_y += EXIT_BTN_HEIGHT + 6.0;
        }
    }

    BottomOverlayOutcome {
        card_rect,
        emitted_action,
        close_confirm_clicked,
        open_confirm_clicked,
        focused_any,
    }
}

fn paint_overlay_button(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    id: Id,
    label: &str,
    style: OverlayButtonStyle,
    pal: &UiPalette,
) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));

    let hovered = resp.hovered();
    let dark = ui.visuals().dark_mode;
    let (bg, border, fg) = match style {
        OverlayButtonStyle::Primary | OverlayButtonStyle::SmallPrimary => {
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
        OverlayButtonStyle::Standard | OverlayButtonStyle::Small => (
            if hovered {
                pal.surface_3
            } else {
                Color32::TRANSPARENT
            },
            pal.border_strong,
            pal.text,
        ),
        OverlayButtonStyle::Danger => (
            if hovered {
                with_alpha(pal.danger, 0.25)
            } else {
                with_alpha(pal.danger, 0.12)
            },
            with_alpha(pal.danger, 0.65),
            if dark { DANGER_TEXT_DARK } else { pal.danger },
        ),
    };

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

    let is_small = matches!(
        style,
        OverlayButtonStyle::Small | OverlayButtonStyle::SmallPrimary
    );
    let font_px = if is_small { 11.5 } else { 12.0 };
    let galley = layout_wrapped_text(ui, label, font_px, fg, (rect.width() - 10.0).max(8.0), 1);
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

fn paint_dismiss_button(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    id: Id,
    label: &str,
    pal: &UiPalette,
) -> Response {
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
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
    icons::paint(painter, icon_rect, Icon::Close, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(label)
}

fn active_palette(ui: &Ui) -> UiPalette {
    if ui.visuals().dark_mode {
        let id = ui
            .ctx()
            .data(|d| d.get_temp::<ThemeId>(Id::new("tessel-theme")))
            .unwrap_or_default();
        id.spec().ui
    } else {
        let v = ui.visuals();
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
