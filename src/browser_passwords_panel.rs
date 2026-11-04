//! Native `egui` Saved Passwords dialog and toolbar trigger (`BrowserPasswords.vue` parity).
//!
//! Security & architectural invariants:
//! - Operates strictly on password-free [`VaultEntryMetadata`] and derived view types from
//!   [`crate::browser_passwords_model`]; no struct, state field, or emitted [`BrowserPasswordsAction`]
//!   ever holds or touches a secret.
//! - Pure UI state & rendering: zero filesystem, SQLite, DPAPI, IPC, or WebView2 side effects.
//! - Enforces confirmation before deleting a login or enabling bulk `allow_agents = true` across
//!   displayed accounts, while disabling bulk `allow_agents = false` applies immediately
//!   (`BrowserPasswords.vue:133-168`).
//! - Intercepts `Escape` in strict priority order (open profile popup → active confirmation →
//!   non-empty search query → close modal) before `egui::TextEdit` or `egui::Modal::should_close`.

use eframe::egui::{
    self, Align, Color32, CornerRadius, CursorIcon, EventFilter, FontId, Id, Key, Margin,
    Modifiers, Order, Painter, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind,
    TextEdit, Ui, Vec2, WidgetInfo, WidgetType,
};

use crate::browser_passwords_model::{
    BatchProfileResultRow, BulkAllowDecision, ConfirmationPrompt, EscapeAction, PAGE_SIZE,
    PasswordsDialogView, PasswordsLang, VaultEntryMetadata, VaultRowView, build_dialog_view,
    decide_escape_action, export_steps_for_source, no_username_placeholder, plan_bulk_allow_change,
    plan_remove_confirmation, row_allow_aria_label, row_remove_aria_label,
};
use crate::icons::{self, Icon};
use crate::theme_palettes::{ThemeId, UiPalette};

const NARROW_BREAKPOINT: f32 = 560.0;
const CONTROL_HEIGHT: f32 = 28.0;
const SMALL_BTN_HEIGHT: f32 = 24.0;
const ICON_BTN_SIDE: f32 = 26.0;
const SWITCH_WIDTH: f32 = 34.0;
const SWITCH_HEIGHT: f32 = 20.0;
const POPUP_OPTION_HEIGHT: f32 = 28.0;
const POPUP_MAX_HEIGHT: f32 = 260.0;
const POPUP_PADDING: f32 = 4.0;
const POPUP_PADDING_I8: i8 = 4;
const OK_CHECK_GREEN: Color32 = Color32::from_rgb(0x4c, 0xaf, 0x7a);
const DANGER_TEXT_TINT: Color32 = Color32::from_rgb(0xff, 0xb4, 0xb6);
const PRIMARY_BTN_FG: Color32 = Color32::from_rgb(0x0b, 0x15, 0x30);

/// Flattened browser profile entry (`BrowserPasswords.vue:84`:
/// `{ browserId: b.id, family: b.family, profileDir: p.dir, label: "${b.name} — ${p.name}" }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserProfileSource {
    pub browser_id: String,
    pub family: String,
    pub profile_dir: String,
    pub label: String,
}

/// Live sequential multi-profile import counter (`Importing {{done}} of {{total}}…`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportProgress {
    pub done: usize,
    pub total: usize,
}

/// Status line shown under the search bar (`.pw-message` / `.pw-message.warn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusBanner<'a> {
    pub text: &'a str,
    pub warn: bool,
}

/// Multi-browser import summary panel (`.pw-results`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchResultsInput<'a> {
    pub title: &'a str,
    pub items: &'a [BatchProfileResultRow],
}

/// Password-free frame input for [`BrowserPasswordsPanel::show`].
#[derive(Debug, Clone, Copy)]
pub struct BrowserPasswordsInput<'a> {
    /// Monotonic revision of the vault snapshot backing `entries`.
    /// Any bump (import, removal, or permission change) invalidates pending confirmations.
    pub vault_revision: u64,
    pub entries: &'a [VaultEntryMetadata],
    pub sources: &'a [BrowserProfileSource],
    /// True when an active private browser page is available (`props.guestId != null`).
    pub can_fill_current_page: bool,
    /// True only when persisted per-login `allow_agents` consent and agent vault tools are wired.
    /// When `false` (owner-only runtime), agent switches, bulk-allow controls, and misleading
    /// copy are suppressed so the UI never offers fictitious permissions.
    pub can_manage_agent_access: bool,
    /// Whether the caller implements direct browser database import.
    pub browser_import_available: bool,
    /// Embedded settings use the parent scroll area and explicit lock control.
    pub embedded: bool,
    pub busy: bool,
    pub progress: Option<ImportProgress>,
    pub status: Option<StatusBanner<'a>>,
    pub batch_results: Option<BatchResultsInput<'a>>,
    pub french: bool,
}

/// Password-free user intent emitted by [`BrowserPasswordsPanel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowserPasswordsAction {
    /// Close the Saved Passwords dialog (`X` button or `Escape` when query & confirmation are empty).
    Close,
    /// Import sequentially from every discovered browser profile (`pw-import-all`).
    ImportAllProfiles,
    /// Import from the selected browser profile (`pw-import-one`).
    ImportProfile {
        source_index: usize,
        browser_id: String,
        profile_dir: String,
    },
    /// Open the local CSV file dialog and import (`pw-import-csv`).
    ImportCsv,
    /// Hide the `.pw-results` batch summary block.
    DismissBatchResults,
    /// Toggle per-login `allow_agents` (only emitted when `can_manage_agent_access == true`).
    SetAllowAgents {
        id: String,
        allow_agents: bool,
        vault_revision: u64,
    },
    /// Apply bulk `allow_agents` across `target_ids` (immediate for `false`, confirmed for `true`;
    /// only emitted when `can_manage_agent_access == true`).
    SetBulkAllowAgents {
        target_ids: Vec<String>,
        allow_agents: bool,
        vault_revision: u64,
    },
    /// Fill the login into the active private browser tab without submitting.
    Fill { id: String, vault_revision: u64 },
    /// Remove the login from Tessel's vault (always preceded by explicit confirmation).
    Delete { id: String, vault_revision: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingConfirmation {
    Delete {
        id: String,
        vault_revision: u64,
        prompt: ConfirmationPrompt,
    },
    BulkAllow {
        target_ids: Vec<String>,
        vault_revision: u64,
        prompt: ConfirmationPrompt,
    },
}

impl PendingConfirmation {
    const fn prompt(&self) -> &ConfirmationPrompt {
        match self {
            Self::Delete { prompt, .. } | Self::BulkAllow { prompt, .. } => prompt,
        }
    }

    const fn vault_revision(&self) -> u64 {
        match self {
            Self::Delete { vault_revision, .. } | Self::BulkAllow { vault_revision, .. } => {
                *vault_revision
            }
        }
    }
}

#[derive(Debug, Clone)]
struct OpenProfilePopup {
    active_index: usize,
    scroll_to_active: bool,
    opened_pass: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalIcon {
    KeyRound,
    Download,
    FileUp,
    Trash2,
    TriangleAlert,
    Check,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitButtonStyle {
    Standard,
    Primary,
    Small,
    SmallDanger,
}

/// Pure `egui` state for Tessel's Saved Passwords modal (`BrowserPasswords.vue`).
#[derive(Debug)]
pub struct BrowserPasswordsPanel {
    query: String,
    last_query: String,
    limit: usize,
    selected_source: Option<usize>,
    selected_profile_key: Option<(String, String)>,
    observed_vault_revision: Option<u64>,
    export_help_open: bool,
    export_browser: usize,
    pending_confirm: Option<PendingConfirmation>,
    profile_popup: Option<OpenProfilePopup>,
    focus_search_next: bool,
    escape_held: bool,
    scroll_epoch: u64,
}

impl Default for BrowserPasswordsPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserPasswordsPanel {
    #[must_use]
    pub fn new() -> Self {
        Self {
            query: String::new(),
            last_query: String::new(),
            limit: PAGE_SIZE,
            selected_source: None,
            selected_profile_key: None,
            observed_vault_revision: None,
            export_help_open: false,
            export_browser: 0,
            pending_confirm: None,
            profile_popup: None,
            focus_search_next: true,
            escape_held: false,
            scroll_epoch: 0,
        }
    }

    /// Resets transient dialog state when opening the Saved Passwords modal (`show()` in Vue).
    pub fn reset_on_open(&mut self) {
        self.query.clear();
        self.last_query.clear();
        self.limit = PAGE_SIZE;
        self.observed_vault_revision = None;
        self.export_help_open = false;
        self.pending_confirm = None;
        self.profile_popup = None;
        self.focus_search_next = true;
        self.escape_held = false;
        self.scroll_epoch = self.scroll_epoch.wrapping_add(1);
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        let next = query.into();
        if next != self.query {
            self.query = next;
            self.last_query.clone_from(&self.query);
            self.limit = PAGE_SIZE;
            self.scroll_epoch = self.scroll_epoch.wrapping_add(1);
        }
    }

    #[must_use]
    pub const fn limit(&self) -> usize {
        self.limit
    }

    #[must_use]
    pub const fn selected_source(&self) -> Option<usize> {
        self.selected_source
    }

    pub fn set_selected_source(&mut self, index: Option<usize>) {
        self.selected_source = index;
        self.selected_profile_key = None;
    }

    #[must_use]
    pub fn selected_profile_identity(&self) -> Option<(&str, &str)> {
        self.selected_profile_key
            .as_ref()
            .map(|(b, p)| (b.as_str(), p.as_str()))
    }

    #[must_use]
    pub const fn observed_vault_revision(&self) -> Option<u64> {
        self.observed_vault_revision
    }

    /// Explicitly cancels any pending `Delete` or `BulkAllow` confirmation (also triggered
    /// automatically whenever [`BrowserPasswordsInput::vault_revision`] changes).
    pub fn invalidate_pending_confirmation(&mut self) {
        self.pending_confirm = None;
    }

    #[must_use]
    pub const fn export_help_open(&self) -> bool {
        self.export_help_open
    }

    pub fn set_export_help_open(&mut self, open: bool) {
        self.export_help_open = open;
    }

    #[must_use]
    pub const fn is_profile_popup_open(&self) -> bool {
        self.profile_popup.is_some()
    }

    #[must_use]
    pub const fn has_pending_confirmation(&self) -> bool {
        self.pending_confirm.is_some()
    }

    #[must_use]
    pub fn pending_confirmation_prompt(&self) -> Option<&ConfirmationPrompt> {
        self.pending_confirm
            .as_ref()
            .map(PendingConfirmation::prompt)
    }

    /// Renders the `.bp-btn` browser-toolbar trigger button (`KeyRound` 16 px).
    pub fn show_trigger_button(ui: &mut Ui, french: bool) -> Response {
        let label = dialog_title(french);
        let enabled = ui.is_enabled();
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(CONTROL_HEIGHT), Sense::hover());
        let id = ui.id().with("bp-trigger-btn");
        let resp = ui.interact(
            rect,
            id,
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));

        let pal = active_palette(ui);
        if enabled && resp.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(6), pal.surface_3);
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(
                rect.expand(1.0),
                CornerRadius::same(6),
                Stroke::new(2.0_f32, pal.accent),
                StrokeKind::Outside,
            );
        }
        let icon_color = if !enabled {
            with_alpha(pal.text_dim, 0.5)
        } else if resp.hovered() {
            pal.text_strong
        } else {
            pal.text_dim
        };
        let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
        paint_local_icon(ui.painter(), icon_rect, LocalIcon::KeyRound, icon_color);
        let resp = if enabled {
            resp.on_hover_cursor(CursorIcon::PointingHand)
        } else {
            resp
        };
        resp.on_hover_text(label)
    }

    /// Handles `Escape` before `egui::TextEdit` or `egui::Modal::should_close` can consume it:
    /// 1. Closes the profile dropdown popup if open.
    /// 2. Cancels any pending confirmation if open.
    /// 3. Clears `query` if non-empty (keeping focus on the search input).
    /// 4. Otherwise emits [`BrowserPasswordsAction::Close`].
    pub fn handle_escape(&mut self, ctx: &egui::Context) -> Option<BrowserPasswordsAction> {
        self.handle_escape_with_search_id(ctx, None)
    }

    fn handle_escape_with_search_id(
        &mut self,
        ctx: &egui::Context,
        search_id: Option<Id>,
    ) -> Option<BrowserPasswordsAction> {
        let down = ctx.input(|i| i.key_down(Key::Escape));
        if !down {
            self.escape_held = false;
        }
        if !ctx.input(|i| i.focused) || egui::Popup::is_any_open(ctx) {
            return None;
        }
        let fresh = ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(event,
            egui::Event::Key { key: Key::Escape, pressed: true, repeat: false, modifiers, .. }
            if modifiers.matches_logically(Modifiers::NONE))
            })
        });
        let consumed = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if !consumed {
            return None;
        }
        if self.escape_held || !fresh {
            if let Some(id) = search_id {
                ctx.memory_mut(|m| m.request_focus(id));
            }
            return None;
        }
        self.escape_held = down;

        if self.profile_popup.is_some() {
            self.profile_popup = None;
            return None;
        }
        if self.pending_confirm.is_some() {
            self.pending_confirm = None;
            return None;
        }
        match decide_escape_action(&self.query) {
            EscapeAction::ClearSearchQuery => {
                self.query.clear();
                self.last_query.clear();
                self.limit = PAGE_SIZE;
                self.scroll_epoch = self.scroll_epoch.wrapping_add(1);
                if let Some(id) = search_id {
                    ctx.memory_mut(|m| m.request_focus(id));
                } else {
                    self.focus_search_next = true;
                }
                None
            }
            EscapeAction::CloseModal => Some(BrowserPasswordsAction::Close),
        }
    }

    /// Renders the Saved Passwords dialog surface (`.pw-card`) and returns any user-initiated action.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        input: BrowserPasswordsInput<'_>,
    ) -> Option<BrowserPasswordsAction> {
        let lang = if input.french {
            PasswordsLang::Fr
        } else {
            PasswordsLang::En
        };
        let panel_id = ui.id().with("tessel-browser-passwords-panel");
        let search_id = panel_id.with("pw-search-input");

        if self
            .observed_vault_revision
            .is_some_and(|prev| prev != input.vault_revision)
        {
            self.invalidate_pending_confirmation();
        }
        self.observed_vault_revision = Some(input.vault_revision);
        if !input.can_manage_agent_access
            && matches!(
                self.pending_confirm,
                Some(PendingConfirmation::BulkAllow { .. })
            )
        {
            self.invalidate_pending_confirmation();
        }

        self.sync_selected_source(input.sources);
        if input.busy {
            self.profile_popup = None;
        }

        let mut action = if ui.is_enabled() && !input.embedded {
            self.handle_escape_with_search_id(ui.ctx(), Some(search_id))
        } else {
            None
        };

        if self.query != self.last_query {
            self.last_query.clone_from(&self.query);
            self.limit = PAGE_SIZE;
            self.scroll_epoch = self.scroll_epoch.wrapping_add(1);
        }

        let view = build_dialog_view(input.entries, &self.query, self.limit, input.busy, lang);
        let pal = active_palette(ui);
        let avail_w = ui.available_width().max(220.0);
        let narrow = avail_w <= NARROW_BREAKPOINT;

        // 1. Header (`.pw-head`)
        if let Some(a) = render_header(ui, panel_id, &pal, input.french, input.embedded)
            && action.is_none()
        {
            action = Some(a);
        }

        // 2. Top controls bar (`.pw-bar`)
        if let Some(a) = self.render_top_bar(
            ui, panel_id, search_id, &pal, &input, &view, narrow, avail_w,
        ) && action.is_none()
        {
            action = Some(a);
        }

        paint_hline(ui, pal.border);

        // 3. Inline confirmation banner (`askConfirm` parity for Delete and Bulk Allow)
        if let Some(a) = self.render_confirmation_card(
            ui,
            panel_id,
            &pal,
            input.vault_revision,
            input.busy,
            input.french,
        ) && action.is_none()
        {
            action = Some(a);
        }

        // 4. Saved accounts list (`.pw-list`)
        if let Some(a) = self.render_accounts_list(ui, panel_id, &pal, &input, &view, lang, narrow)
            && action.is_none()
        {
            action = Some(a);
        }

        paint_hline(ui, pal.border);

        // 5. Footer & expandable CSV export guide (`.pw-foot`)
        self.render_footer(ui, panel_id, &pal, &input, lang);

        action
    }

    fn select_source_in_list(&mut self, index: usize, sources: &[BrowserProfileSource]) {
        if sources.is_empty() {
            self.selected_source = None;
            self.selected_profile_key = None;
            return;
        }
        let clamped = index.min(sources.len() - 1);
        let src = &sources[clamped];
        self.selected_source = Some(clamped);
        self.selected_profile_key = Some((src.browser_id.clone(), src.profile_dir.clone()));
    }

    fn sync_selected_source(&mut self, sources: &[BrowserProfileSource]) {
        if sources.is_empty() {
            self.selected_source = None;
            self.selected_profile_key = None;
            self.profile_popup = None;
            return;
        }

        if let Some((ref bid, ref pdir)) = self.selected_profile_key {
            if let Some(found_idx) = sources
                .iter()
                .position(|s| s.browser_id == *bid && s.profile_dir == *pdir)
            {
                if self.selected_source != Some(found_idx) {
                    self.selected_source = Some(found_idx);
                    self.profile_popup = None;
                } else if self
                    .profile_popup
                    .as_ref()
                    .is_some_and(|p| p.active_index >= sources.len())
                {
                    self.profile_popup = None;
                }
            } else {
                // Previously selected `(browser_id, profile_dir)` disappeared on refresh: reset cleanly.
                self.select_source_in_list(0, sources);
                self.profile_popup = None;
            }
            return;
        }

        let initial_idx = self
            .selected_source
            .filter(|&idx| idx < sources.len())
            .unwrap_or(0);
        self.select_source_in_list(initial_idx, sources);
        if self
            .profile_popup
            .as_ref()
            .is_some_and(|p| p.active_index >= sources.len())
        {
            self.profile_popup = None;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_top_bar(
        &mut self,
        ui: &mut Ui,
        panel_id: Id,
        search_id: Id,
        pal: &UiPalette,
        input: &BrowserPasswordsInput<'_>,
        view: &PasswordsDialogView,
        narrow: bool,
        avail_w: f32,
    ) -> Option<BrowserPasswordsAction> {
        let mut action = None;
        let fr = input.french;

        egui::Frame::NONE
            .inner_margin(Margin::symmetric(16, 6))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);

                // `.pw-import`
                let row_avail = ui.available_width();
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);

                    let import_all_text = if let Some(prog) = input.progress {
                        importing_progress_label(prog, fr)
                    } else {
                        import_all_label(fr).to_owned()
                    };
                    let import_one_text = import_one_label(fr);
                    let import_csv_text = import_csv_label(fr);

                    let can_import_all =
                        ui.is_enabled() && !input.busy && input.browser_import_available && !input.sources.is_empty();
                    if render_exit_button(
                        ui,
                        panel_id.with("pw-import-all"),
                        &import_all_text,
                        Some(LocalIcon::Download),
                        ExitButtonStyle::Primary,
                        can_import_all,
                        pal,
                    )
                    .clicked()
                    {
                        action = Some(BrowserPasswordsAction::ImportAllProfiles);
                    }

                    let select_w = if narrow {
                        (avail_w - 32.0).max(140.0)
                    } else {
                        let w_all = measure_exit_button_width(ui, &import_all_text, true, false);
                        let w_one = measure_exit_button_width(ui, import_one_text, false, false);
                        let w_csv = measure_exit_button_width(ui, import_csv_text, true, false);
                        let flex_w = row_avail - w_all - w_one - w_csv - 24.0;
                        if flex_w >= 140.0 { flex_w } else { 196.0 }
                    };
                    self.render_profile_select(
                        ui,
                        panel_id.with("pw-source"),
                        select_w,
                        input.sources,
                        ui.is_enabled() && !input.busy && input.browser_import_available && !input.sources.is_empty(),
                        fr,
                        pal,
                    );

                    let can_import_one =
                        ui.is_enabled() && !input.busy && input.browser_import_available && self.selected_source.is_some();
                    if render_exit_button(
                        ui,
                        panel_id.with("pw-import-one"),
                        import_one_text,
                        None,
                        ExitButtonStyle::Standard,
                        can_import_one,
                        pal,
                    )
                    .clicked()
                        && let Some(idx) = self.selected_source
                        && let Some(src) = input.sources.get(idx)
                    {
                        action = Some(BrowserPasswordsAction::ImportProfile {
                            source_index: idx,
                            browser_id: src.browser_id.clone(),
                            profile_dir: src.profile_dir.clone(),
                        });
                    }

                    let can_import_csv = ui.is_enabled() && !input.busy;
                    if render_exit_button(
                        ui,
                        panel_id.with("pw-import-csv"),
                        import_csv_text,
                        Some(LocalIcon::FileUp),
                        ExitButtonStyle::Standard,
                        can_import_csv,
                        pal,
                    )
                    .clicked()
                    {
                        action = Some(BrowserPasswordsAction::ImportCsv);
                    }
                });

                // Empty browser-sources hint
                if !input.busy && (!input.browser_import_available || input.sources.is_empty()) {
                    ui.add(
                        egui::Label::new(
                            RichText::new(if input.browser_import_available { no_sources_label(fr) } else if fr { "L’import direct du navigateur n’est pas encore disponible. Utilisez un CSV de mots de passe exporté par votre navigateur." } else { "Direct browser import is not available yet. Use a password CSV exported by your browser." })
                                .size(12.0)
                                .color(pal.text_dim),
                        )
                        .wrap(),
                    );
                }

                // `.pw-search-row`
                let (agents_badge, agents_tip) = if input.can_manage_agent_access {
                    (view.agents_label.as_str(), agents_hint_label(fr))
                } else {
                    (
                        agents_owner_only_label(fr),
                        agents_owner_only_hint_label(fr),
                    )
                };
                if narrow {
                    self.render_search_box(ui, search_id, (avail_w - 32.0).max(120.0), fr, pal);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        ui.label(
                            RichText::new(&view.count_label)
                                .size(12.0)
                                .color(pal.text_dim),
                        );
                        if view.total_rows > 0 {
                            ui.add(egui::Label::new(
                                RichText::new(agents_badge).size(12.0).color(pal.text_dim),
                            ))
                            .on_hover_text(agents_tip);
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let count_galley = ui.painter().layout_no_wrap(
                            view.count_label.clone(),
                            FontId::proportional(12.0),
                            pal.text_dim,
                        );
                        let agents_w = if view.total_rows > 0 {
                            ui.painter()
                                .layout_no_wrap(
                                    agents_badge.to_owned(),
                                    FontId::proportional(12.0),
                                    pal.text_dim,
                                )
                                .size()
                                .x
                                + 16.0
                        } else {
                            0.0
                        };
                        let search_w =
                            (ui.available_width() - count_galley.size().x - agents_w - 16.0)
                                .max(180.0);
                        self.render_search_box(ui, search_id, search_w, fr, pal);
                        ui.label(
                            RichText::new(&view.count_label)
                                .size(12.0)
                                .color(pal.text_dim),
                        );
                        if view.total_rows > 0 {
                            ui.add(egui::Label::new(
                                RichText::new(agents_badge).size(12.0).color(pal.text_dim),
                            ))
                            .on_hover_text(agents_tip);
                        }
                    });
                }

                // `.pw-message`
                if let Some(status) = input.status
                    && !status.text.is_empty()
                {
                    let msg_color = if status.warn { pal.warn } else { pal.text };
                    ui.add(
                        egui::Label::new(RichText::new(status.text).size(12.0).color(msg_color))
                            .wrap(),
                    );
                }

                // `.pw-results`
                if let Some(batch) = input.batch_results
                    && !batch.items.is_empty()
                    && let Some(a) = self.render_batch_results(
                        ui,
                        panel_id.with("pw-results"),
                        pal,
                        batch,
                        input.progress,
                        input.sources,
                        input.busy,
                        fr,
                    )
                {
                    action = Some(a);
                }
            });

        action
    }

    fn render_search_box(
        &mut self,
        ui: &mut Ui,
        search_id: Id,
        width: f32,
        french: bool,
        pal: &UiPalette,
    ) {
        let placeholder = search_placeholder(french);
        let (box_rect, _) =
            ui.allocate_exact_size(Vec2::new(width, CONTROL_HEIGHT), Sense::hover());
        let has_focus = ui.memory(|m| m.has_focus(search_id));
        let border_col = if has_focus {
            pal.accent
        } else {
            pal.border_strong
        };
        ui.painter().rect(
            box_rect,
            CornerRadius::same(6),
            pal.term,
            Stroke::new(1.0_f32, border_col),
            StrokeKind::Inside,
        );

        let icon_rect = Rect::from_center_size(
            Pos2::new(box_rect.left() + 14.0, box_rect.center().y),
            Vec2::splat(13.0),
        );
        icons::paint(ui.painter(), icon_rect, Icon::Search, pal.text_dim);

        let edit_rect = Rect::from_min_max(
            Pos2::new(box_rect.left() + 26.0, box_rect.top() + 3.0),
            Pos2::new(
                (box_rect.right() - 6.0).max(box_rect.left() + 30.0),
                box_rect.bottom() - 3.0,
            ),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(edit_rect)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        let resp = child.add(
            TextEdit::singleline(&mut self.query)
                .id(search_id)
                .frame(false)
                .font(FontId::proportional(12.0))
                .text_color(pal.text_strong)
                .hint_text(RichText::new(placeholder).size(12.0).color(pal.text_dim))
                .desired_width(edit_rect.width()),
        );

        if self.focus_search_next && ui.is_enabled() && !ui.is_sizing_pass() {
            if ui.memory(|m| m.focused().is_none_or(|id| id == search_id)) {
                resp.request_focus();
            }
            self.focus_search_next = false;
        }
        if resp.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    search_id,
                    EventFilter {
                        escape: true,
                        ..Default::default()
                    },
                );
            });
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_profile_select(
        &mut self,
        ui: &mut Ui,
        trigger_id: Id,
        width: f32,
        sources: &[BrowserProfileSource],
        enabled: bool,
        french: bool,
        pal: &UiPalette,
    ) {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, CONTROL_HEIGHT), Sense::hover());
        let resp = ui.interact(
            rect,
            trigger_id,
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let selected_idx = self.selected_source.unwrap_or(0);
        let selected_text = sources
            .get(selected_idx)
            .map_or_else(|| profile_label(french), |s| s.label.as_str());
        let a11y_label = format!("{}: {selected_text}", profile_label(french));
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, enabled, &a11y_label));

        let is_open = self.profile_popup.is_some();
        if enabled && resp.clicked() {
            resp.request_focus();
            if is_open {
                self.profile_popup = None;
            } else {
                self.profile_popup = Some(OpenProfilePopup {
                    active_index: selected_idx.min(sources.len().saturating_sub(1)),
                    scroll_to_active: true,
                    opened_pass: ui.ctx().cumulative_pass_nr(),
                });
            }
        }

        if enabled && resp.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    trigger_id,
                    EventFilter {
                        escape: true,
                        vertical_arrows: self.profile_popup.is_some(),
                        ..Default::default()
                    },
                );
            });
            self.handle_profile_select_keys(ui.ctx(), sources, selected_idx);
        }

        let focused_or_open = resp.has_focus() || self.profile_popup.is_some();
        let border_col = if focused_or_open && enabled {
            pal.accent
        } else {
            pal.border_strong
        };
        let bg = if enabled && resp.hovered() {
            pal.surface_3
        } else {
            pal.term
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(6),
            if enabled { bg } else { with_alpha(bg, 0.5) },
            Stroke::new(
                1.0_f32,
                if enabled {
                    border_col
                } else {
                    with_alpha(border_col, 0.5)
                },
            ),
            StrokeKind::Inside,
        );

        let text_col = if enabled {
            pal.text
        } else {
            with_alpha(pal.text_dim, 0.5)
        };
        let text_rect = Rect::from_min_max(
            Pos2::new(rect.left() + 10.0, rect.top()),
            Pos2::new((rect.right() - 24.0).max(rect.left() + 12.0), rect.bottom()),
        );
        paint_truncated_line(
            ui,
            text_rect,
            selected_text,
            FontId::proportional(12.0),
            text_col,
        );

        let chev_rect = Rect::from_center_size(
            Pos2::new(rect.right() - 12.0, rect.center().y),
            Vec2::splat(12.0),
        );
        icons::paint(
            ui.painter(),
            chev_rect,
            Icon::ChevronDown,
            if enabled {
                pal.text_dim
            } else {
                with_alpha(pal.text_dim, 0.5)
            },
        );
        if enabled {
            let _ = resp.on_hover_cursor(CursorIcon::PointingHand);
        }

        if self.profile_popup.is_some() && !sources.is_empty() {
            self.render_profile_popup(ui.ctx(), trigger_id, rect, sources, selected_idx, pal);
        }
    }

    fn handle_profile_select_keys(
        &mut self,
        ctx: &egui::Context,
        sources: &[BrowserProfileSource],
        selected_idx: usize,
    ) {
        if sources.is_empty() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            if let Some(open) = self.profile_popup.as_mut() {
                if open.active_index + 1 < sources.len() {
                    open.active_index += 1;
                    open.scroll_to_active = true;
                }
            } else {
                self.profile_popup = Some(OpenProfilePopup {
                    active_index: selected_idx.min(sources.len() - 1),
                    scroll_to_active: true,
                    opened_pass: ctx.cumulative_pass_nr(),
                });
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp))
            && let Some(open) = self.profile_popup.as_mut()
            && open.active_index > 0
        {
            open.active_index -= 1;
            open.scroll_to_active = true;
        }
        if (ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
            || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space)))
            && let Some(open) = self.profile_popup.take()
        {
            self.select_source_in_list(open.active_index, sources);
        }
    }

    fn render_profile_popup(
        &mut self,
        ctx: &egui::Context,
        trigger_id: Id,
        trigger_rect: Rect,
        sources: &[BrowserProfileSource],
        selected_idx: usize,
        pal: &UiPalette,
    ) {
        let popup_w = trigger_rect.width().max(240.0);
        let popup_h = ((sources.len() as f32) * POPUP_OPTION_HEIGHT + POPUP_PADDING * 2.0)
            .min(POPUP_MAX_HEIGHT);
        let screen = ctx.content_rect();
        let x = trigger_rect
            .left()
            .min((screen.right() - popup_w - 4.0).max(screen.left()));
        let y = if trigger_rect.bottom() + 4.0 + popup_h <= screen.bottom() {
            trigger_rect.bottom() + 4.0
        } else {
            (trigger_rect.top() - 4.0 - popup_h).max(screen.top() + 4.0)
        };
        let popup_rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(popup_w, popup_h));

        let pointer_moved = ctx.input(|i| i.pointer.delta() != Vec2::ZERO);
        let pass_nr = ctx.cumulative_pass_nr();
        let (mut active, mut scroll_to_active, opened_pass) = self
            .profile_popup
            .as_ref()
            .map_or((selected_idx, false, pass_nr), |p| {
                (p.active_index, p.scroll_to_active, p.opened_pass)
            });
        let mut chosen: Option<usize> = None;

        egui::Area::new(trigger_id.with("pw-source-popup"))
            .order(Order::Tooltip)
            .fixed_pos(popup_rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                let sizing_pass = ui.is_sizing_pass();
                egui::Frame::NONE
                    .fill(pal.surface_2)
                    .stroke(Stroke::new(1.0_f32, pal.border_strong))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(POPUP_PADDING_I8))
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        let inner_w = (popup_w - POPUP_PADDING * 2.0).max(40.0);
                        let inner_h = (popup_h - POPUP_PADDING * 2.0).max(24.0);
                        ui.set_min_width(inner_w);
                        ui.set_max_width(inner_w);
                        egui::ScrollArea::vertical()
                            .id_salt(trigger_id.with("pw-source-scroll"))
                            .max_height(inner_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for (idx, src) in sources.iter().enumerate() {
                                    let (row_rect, _) = ui.allocate_exact_size(
                                        Vec2::new(inner_w, POPUP_OPTION_HEIGHT),
                                        Sense::hover(),
                                    );
                                    let opt_resp = ui.interact(
                                        row_rect,
                                        trigger_id.with(("pw-source-opt", idx)),
                                        Sense::click(),
                                    );
                                    let is_selected = idx == selected_idx;
                                    opt_resp.widget_info(|| {
                                        WidgetInfo::selected(
                                            WidgetType::SelectableLabel,
                                            true,
                                            is_selected,
                                            &src.label,
                                        )
                                    });
                                    if pointer_moved && opt_resp.hovered() {
                                        active = idx;
                                    }
                                    if scroll_to_active && idx == active && !sizing_pass {
                                        opt_resp.scroll_to_me(Some(Align::Center));
                                        scroll_to_active = false;
                                    }
                                    if idx == active || opt_resp.hovered() {
                                        ui.painter().rect_filled(
                                            row_rect,
                                            CornerRadius::same(4),
                                            pal.surface_3,
                                        );
                                    }
                                    let text_rect = Rect::from_min_max(
                                        Pos2::new(row_rect.left() + 8.0, row_rect.top()),
                                        Pos2::new(
                                            (row_rect.right() - 24.0).max(row_rect.left() + 10.0),
                                            row_rect.bottom(),
                                        ),
                                    );
                                    paint_truncated_line(
                                        ui,
                                        text_rect,
                                        &src.label,
                                        FontId::proportional(12.0),
                                        if is_selected {
                                            pal.text_strong
                                        } else {
                                            pal.text
                                        },
                                    );
                                    if is_selected {
                                        let chk_rect = Rect::from_center_size(
                                            Pos2::new(row_rect.right() - 12.0, row_rect.center().y),
                                            Vec2::splat(13.0),
                                        );
                                        paint_local_icon(
                                            ui.painter(),
                                            chk_rect,
                                            LocalIcon::Check,
                                            pal.accent,
                                        );
                                    }
                                    if opt_resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
                                    {
                                        chosen = Some(idx);
                                    }
                                }
                            });
                    });
            });

        if let Some(idx) = chosen {
            self.select_source_in_list(idx, sources);
            self.profile_popup = None;
            ctx.memory_mut(|m| m.request_focus(trigger_id));
            return;
        }

        let clicked_outside = opened_pass != pass_nr
            && ctx.input(|i| {
                i.pointer.primary_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|pos| !popup_rect.contains(pos) && !trigger_rect.contains(pos))
            });
        if clicked_outside {
            self.profile_popup = None;
        } else if let Some(open) = self.profile_popup.as_mut() {
            open.active_index = active;
            open.scroll_to_active = scroll_to_active;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_batch_results(
        &mut self,
        ui: &mut Ui,
        results_id: Id,
        pal: &UiPalette,
        batch: BatchResultsInput<'_>,
        progress: Option<ImportProgress>,
        sources: &[BrowserProfileSource],
        busy: bool,
        french: bool,
    ) -> Option<BrowserPasswordsAction> {
        let mut action = None;
        let fallback_progress = progress.map(|p| importing_progress_label(p, french));
        let heading = if !batch.title.is_empty() {
            batch.title
        } else {
            fallback_progress.as_deref().unwrap_or("")
        };

        egui::Frame::NONE
            .fill(pal.surface_2)
            .stroke(Stroke::new(1.0_f32, pal.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    let head_w = (ui.available_width() - ICON_BTN_SIDE - 8.0).max(60.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(head_w, ICON_BTN_SIDE),
                        egui::Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(heading)
                                        .size(12.0)
                                        .strong()
                                        .color(pal.text_strong),
                                )
                                .wrap(),
                            );
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        let dismiss = dismiss_label(french);
                        if render_icon_button(
                            ui,
                            results_id.with("dismiss"),
                            dismiss,
                            false,
                            ui.is_enabled() && !busy,
                            pal,
                            |painter, rect, col| {
                                icons::paint(painter, rect, Icon::Close, col);
                            },
                            14.0,
                        )
                        .clicked()
                        {
                            action = Some(BrowserPasswordsAction::DismissBatchResults);
                        }
                    });
                });

                egui::ScrollArea::vertical()
                    .id_salt(results_id.with("scroll"))
                    .max_height(150.0)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for item in batch.items {
                            paint_hline(ui, pal.border);
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                let (icon_rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                                if item.warn {
                                    paint_local_icon(
                                        ui.painter(),
                                        icon_rect,
                                        LocalIcon::TriangleAlert,
                                        pal.warn,
                                    );
                                } else {
                                    paint_local_icon(
                                        ui.painter(),
                                        icon_rect,
                                        LocalIcon::Check,
                                        OK_CHECK_GREEN,
                                    );
                                }
                                ui.label(
                                    RichText::new(&item.label)
                                        .size(12.0)
                                        .strong()
                                        .color(pal.text_strong),
                                );
                                ui.label(RichText::new(&item.text).size(12.0).color(pal.text_dim));
                                if item.csv
                                    && render_exit_button(
                                        ui,
                                        results_id.with(("how-export", item.index)),
                                        how_export_label(french),
                                        None,
                                        ExitButtonStyle::Small,
                                        ui.is_enabled(),
                                        pal,
                                    )
                                    .clicked()
                                {
                                    if !sources.is_empty() {
                                        let target_idx = sources
                                            .get(item.index)
                                            .filter(|s| s.label == item.label)
                                            .map(|_| item.index)
                                            .or_else(|| {
                                                sources.iter().position(|s| s.label == item.label)
                                            })
                                            .unwrap_or_else(|| item.index.min(sources.len() - 1));
                                        self.select_source_in_list(target_idx, sources);
                                    }
                                    self.export_help_open = true;
                                }
                            });
                        }
                    });
            });

        action
    }

    fn render_confirmation_card(
        &mut self,
        ui: &mut Ui,
        panel_id: Id,
        pal: &UiPalette,
        vault_revision: u64,
        busy: bool,
        french: bool,
    ) -> Option<BrowserPasswordsAction> {
        let pending = self.pending_confirm.clone()?;
        let prompt = pending.prompt();
        let mut cancel_clicked = false;
        let mut confirm_clicked = false;

        egui::Frame::NONE
            .inner_margin(Margin::symmetric(16, 8))
            .show(ui, |ui| {
                let border_col = if prompt.danger {
                    pal.danger
                } else {
                    pal.accent
                };
                egui::Frame::NONE
                    .fill(pal.surface_2)
                    .stroke(Stroke::new(1.0_f32, border_col))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        ui.add(
                            egui::Label::new(
                                RichText::new(&prompt.title)
                                    .size(13.0)
                                    .strong()
                                    .color(pal.text_strong),
                            )
                            .wrap(),
                        );
                        ui.add(
                            egui::Label::new(
                                RichText::new(&prompt.text).size(12.0).color(pal.text),
                            )
                            .wrap(),
                        );
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let confirm_style = if prompt.danger {
                                    ExitButtonStyle::SmallDanger
                                } else {
                                    ExitButtonStyle::Small
                                };
                                if render_exit_button(
                                    ui,
                                    panel_id.with("confirm-accept"),
                                    &prompt.confirm_label,
                                    None,
                                    confirm_style,
                                    ui.is_enabled() && !busy,
                                    pal,
                                )
                                .clicked()
                                {
                                    confirm_clicked = true;
                                }
                                if render_exit_button(
                                    ui,
                                    panel_id.with("confirm-cancel"),
                                    cancel_label(french),
                                    None,
                                    ExitButtonStyle::Small,
                                    ui.is_enabled(),
                                    pal,
                                )
                                .clicked()
                                {
                                    cancel_clicked = true;
                                }
                            });
                        });
                    });
            });

        paint_hline(ui, pal.border);

        if cancel_clicked {
            self.pending_confirm = None;
            return None;
        }
        if confirm_clicked {
            let confirmed = self.pending_confirm.take()?;
            if confirmed.vault_revision() != vault_revision {
                return None;
            }
            return Some(match confirmed {
                PendingConfirmation::Delete {
                    id, vault_revision, ..
                } => BrowserPasswordsAction::Delete { id, vault_revision },
                PendingConfirmation::BulkAllow {
                    target_ids,
                    vault_revision,
                    ..
                } => BrowserPasswordsAction::SetBulkAllowAgents {
                    target_ids,
                    allow_agents: true,
                    vault_revision,
                },
            });
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    fn render_accounts_list(
        &mut self,
        ui: &mut Ui,
        panel_id: Id,
        pal: &UiPalette,
        input: &BrowserPasswordsInput<'_>,
        view: &PasswordsDialogView,
        lang: PasswordsLang,
        narrow: bool,
    ) -> Option<BrowserPasswordsAction> {
        let mut action = None;
        let fr = input.french;
        let fill_text = fill_short_label(fr);
        let fill_btn_w = measure_small_button_width(ui, fill_text);

        if view.total_rows > 0 && !narrow {
            if let Some(a) = self.render_list_header(
                ui,
                panel_id.with("pw-list-head"),
                pal,
                input,
                view,
                lang,
                fill_btn_w,
            ) {
                action = Some(a);
            }
            paint_hline(ui, pal.border);
        }

        let screen_h = ui.ctx().content_rect().height();
        let max_list_h = (screen_h - 320.0)
            .clamp(140.0, 420.0)
            .min((ui.available_height() - 96.0).max(120.0));
        egui::ScrollArea::vertical()
            .id_salt((panel_id.with("pw-list-scroll"), self.scroll_epoch))
            .max_height(max_list_h)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;

                if !input.busy && view.total_rows == 0 {
                    render_centered_empty_note(ui, empty_vault_label(fr), pal);
                    return;
                }
                if view.total_rows > 0 && view.filtered_count == 0 {
                    render_centered_empty_note(ui, &view.no_match_label, pal);
                    return;
                }

                for row in &view.visible_rows {
                    let row_act = if narrow {
                        self.render_narrow_account_row(
                            ui, panel_id, pal, row, input, lang, fill_text, fill_btn_w,
                        )
                    } else {
                        self.render_wide_account_row(
                            ui, panel_id, pal, row, input, lang, fill_text, fill_btn_w,
                        )
                    };
                    if let Some(a) = row_act
                        && action.is_none()
                    {
                        action = Some(a);
                    }
                    paint_hline(ui, pal.border);
                }

                if view.remaining > 0 {
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        if render_exit_button(
                            ui,
                            panel_id.with("pw-more"),
                            &view.more_label,
                            None,
                            ExitButtonStyle::Small,
                            ui.is_enabled(),
                            pal,
                        )
                        .clicked()
                        {
                            self.limit = self.limit.saturating_add(PAGE_SIZE);
                        }
                    });
                    ui.add_space(10.0);
                }
            });

        action
    }

    #[allow(clippy::too_many_arguments)]
    fn render_list_header(
        &mut self,
        ui: &mut Ui,
        head_id: Id,
        pal: &UiPalette,
        input: &BrowserPasswordsInput<'_>,
        view: &PasswordsDialogView,
        lang: PasswordsLang,
        fill_btn_w: f32,
    ) -> Option<BrowserPasswordsAction> {
        let mut action = None;
        let fr = input.french;
        let row_w = ui.available_width().max(260.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(row_w, 28.0), Sense::hover());
        ui.painter()
            .rect_filled(rect, CornerRadius::ZERO, pal.surface);

        let cols = compute_wide_columns(rect, fill_btn_w, input.can_manage_agent_access);
        let head_font = FontId::proportional(11.0);
        paint_truncated_line(ui, cols.site, "SITE", head_font.clone(), pal.text_dim);
        paint_truncated_line(
            ui,
            cols.user,
            username_col_header(fr),
            head_font.clone(),
            pal.text_dim,
        );

        if let Some(switch_rect) = cols.switch {
            let switch_enabled = ui.is_enabled() && !view.bulk_switch.disabled;
            let bulk_resp = render_switch_pill(
                ui,
                switch_rect,
                head_id.with("bulk-switch"),
                &view.bulk_switch.label,
                view.bulk_switch.checked,
                view.bulk_switch.indeterminate,
                switch_enabled,
                pal,
            );
            if bulk_resp.clicked() {
                let target_allow = !view.bulk_switch.checked;
                match plan_bulk_allow_change(input.entries, &self.query, target_allow, true, lang) {
                    BulkAllowDecision::NoOp | BulkAllowDecision::RefusedWithoutConfirmation => {}
                    BulkAllowDecision::ApplyImmediately {
                        target_ids,
                        allow_agents,
                    } => {
                        self.pending_confirm = None;
                        action = Some(BrowserPasswordsAction::SetBulkAllowAgents {
                            target_ids,
                            allow_agents,
                            vault_revision: input.vault_revision,
                        });
                    }
                    BulkAllowDecision::AskConfirmation { target_ids, prompt } => {
                        self.pending_confirm = Some(PendingConfirmation::BulkAllow {
                            target_ids,
                            vault_revision: input.vault_revision,
                            prompt,
                        });
                    }
                }
            }

            let agents_rect =
                Rect::from_min_max(cols.fill.min, Pos2::new(cols.trash.max.x, rect.max.y));
            paint_truncated_line(ui, agents_rect, "AGENTS", head_font, pal.text_dim);
        }

        action
    }

    #[allow(clippy::too_many_arguments)]
    fn render_wide_account_row(
        &mut self,
        ui: &mut Ui,
        panel_id: Id,
        pal: &UiPalette,
        row: &VaultRowView,
        input: &BrowserPasswordsInput<'_>,
        lang: PasswordsLang,
        fill_text: &str,
        fill_btn_w: f32,
    ) -> Option<BrowserPasswordsAction> {
        let row_w = ui.available_width().max(260.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(row_w, 34.0), Sense::hover());
        if ui.rect_contains_pointer(rect) {
            ui.painter()
                .rect_filled(rect, CornerRadius::ZERO, pal.surface_2);
        }

        let cols = compute_wide_columns(rect, fill_btn_w, input.can_manage_agent_access);
        let site_color = if row.same {
            pal.text_dim
        } else {
            pal.text_strong
        };
        paint_truncated_line(
            ui,
            cols.site,
            &row.site,
            FontId::proportional(12.5),
            site_color,
        );
        ui.interact(
            cols.site,
            panel_id.with(("site-tip", &row.id)),
            Sense::hover(),
        )
        .on_hover_text(&row.origin);

        let user_display = if row.username.is_empty() {
            no_username_placeholder(lang)
        } else {
            row.username.as_str()
        };
        paint_truncated_line(
            ui,
            cols.user,
            user_display,
            FontId::proportional(12.5),
            if row.username.is_empty() {
                pal.text_dim
            } else {
                pal.text
            },
        );
        if !row.username.is_empty() {
            ui.interact(
                cols.user,
                panel_id.with(("user-tip", &row.id)),
                Sense::hover(),
            )
            .on_hover_text(&row.username);
        }

        self.render_row_controls(
            ui,
            panel_id,
            pal,
            row,
            input,
            lang,
            fill_text,
            cols.switch,
            cols.fill,
            cols.trash,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_narrow_account_row(
        &mut self,
        ui: &mut Ui,
        panel_id: Id,
        pal: &UiPalette,
        row: &VaultRowView,
        input: &BrowserPasswordsInput<'_>,
        lang: PasswordsLang,
        fill_text: &str,
        fill_btn_w: f32,
    ) -> Option<BrowserPasswordsAction> {
        let row_w = ui.available_width().max(220.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(row_w, 46.0), Sense::hover());
        if ui.rect_contains_pointer(rect) {
            ui.painter()
                .rect_filled(rect, CornerRadius::ZERO, pal.surface_2);
        }

        let trash_rect = Rect::from_center_size(
            Pos2::new(rect.right() - 16.0 - ICON_BTN_SIDE * 0.5, rect.center().y),
            Vec2::splat(ICON_BTN_SIDE),
        );
        let fill_rect = Rect::from_center_size(
            Pos2::new(trash_rect.left() - 8.0 - fill_btn_w * 0.5, rect.center().y),
            Vec2::new(fill_btn_w, SMALL_BTN_HEIGHT),
        );
        let switch_rect = input.can_manage_agent_access.then(|| {
            Rect::from_center_size(
                Pos2::new(fill_rect.left() - 8.0 - SWITCH_WIDTH * 0.5, rect.center().y),
                Vec2::new(SWITCH_WIDTH, SWITCH_HEIGHT),
            )
        });
        let right_anchor = switch_rect.map_or(fill_rect.left(), |r| r.left());
        let text_max_x = (right_anchor - 8.0).max(rect.left() + 24.0);
        let site_rect = Rect::from_min_max(
            Pos2::new(rect.left() + 16.0, rect.top() + 5.0),
            Pos2::new(text_max_x, rect.center().y),
        );
        let user_rect = Rect::from_min_max(
            Pos2::new(rect.left() + 16.0, rect.center().y),
            Pos2::new(text_max_x, rect.bottom() - 5.0),
        );

        let site_color = if row.same {
            pal.text_dim
        } else {
            pal.text_strong
        };
        paint_truncated_line(
            ui,
            site_rect,
            &row.site,
            FontId::proportional(12.5),
            site_color,
        );
        let user_display = if row.username.is_empty() {
            no_username_placeholder(lang)
        } else {
            row.username.as_str()
        };
        paint_truncated_line(
            ui,
            user_rect,
            user_display,
            FontId::proportional(11.5),
            pal.text_dim,
        );

        self.render_row_controls(
            ui,
            panel_id,
            pal,
            row,
            input,
            lang,
            fill_text,
            switch_rect,
            fill_rect,
            trash_rect,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_row_controls(
        &mut self,
        ui: &mut Ui,
        panel_id: Id,
        pal: &UiPalette,
        row: &VaultRowView,
        input: &BrowserPasswordsInput<'_>,
        lang: PasswordsLang,
        fill_text: &str,
        switch_rect: Option<Rect>,
        fill_rect: Rect,
        trash_rect: Rect,
    ) -> Option<BrowserPasswordsAction> {
        let mut action = None;
        let fr = input.french;
        let remove_label = row_remove_aria_label(&row.site, &row.username, lang);

        if let Some(sw_rect) = switch_rect {
            let allow_label = row_allow_aria_label(&row.site, &row.username, lang);
            let sw_resp = render_switch_pill(
                ui,
                sw_rect,
                panel_id.with(("row-allow", &row.id)),
                &allow_label,
                row.allow_agents,
                false,
                ui.is_enabled() && !input.busy,
                pal,
            );
            if sw_resp.clicked() {
                action = Some(BrowserPasswordsAction::SetAllowAgents {
                    id: row.id.clone(),
                    allow_agents: !row.allow_agents,
                    vault_revision: input.vault_revision,
                });
            }
        }

        let can_fill = ui.is_enabled() && !input.busy && input.can_fill_current_page;
        let fill_resp = render_exit_button_in_rect(
            ui,
            fill_rect,
            panel_id.with(("row-fill", &row.id)),
            fill_text,
            None,
            ExitButtonStyle::Small,
            can_fill,
            pal,
        )
        .on_hover_text(fill_hint_label(fr));
        if fill_resp.clicked() {
            action = Some(BrowserPasswordsAction::Fill {
                id: row.id.clone(),
                vault_revision: input.vault_revision,
            });
        }

        let rm_resp = render_icon_button_in_rect(
            ui,
            trash_rect,
            panel_id.with(("row-rm", &row.id)),
            &remove_label,
            true,
            ui.is_enabled() && !input.busy,
            pal,
            |painter, rect, col| {
                paint_local_icon(painter, rect, LocalIcon::Trash2, col);
            },
            14.0,
        );
        if rm_resp.clicked() {
            let prompt = plan_remove_confirmation(&row.site, &row.username, lang);
            self.pending_confirm = Some(PendingConfirmation::Delete {
                id: row.id.clone(),
                vault_revision: input.vault_revision,
                prompt,
            });
        }

        action
    }

    fn render_footer(
        &mut self,
        ui: &mut Ui,
        panel_id: Id,
        pal: &UiPalette,
        input: &BrowserPasswordsInput<'_>,
        lang: PasswordsLang,
    ) {
        let french = input.french;
        let sources = input.sources;
        let local_note = if input.can_manage_agent_access {
            local_short_label(french)
        } else {
            local_owner_only_label(french)
        };
        egui::Frame::NONE
            .inner_margin(Margin::symmetric(16, 8))
            .show(ui, |ui| {
                let content = |ui: &mut Ui| {
                    ui.spacing_mut().item_spacing.y = 5.0;
                    ui.add(
                        egui::Label::new(RichText::new(local_note).size(11.5).color(pal.text_dim))
                            .wrap(),
                    );

                    let summary_text = export_help_summary(french);
                    let toggle_id = panel_id.with("pw-export-toggle");
                    let arrow = if self.export_help_open { "▾" } else { "▸" };
                    let header_line = format!("{arrow} {summary_text}");
                    let galley = ui.painter().layout(
                        header_line,
                        FontId::proportional(12.0),
                        pal.text_strong,
                        ui.available_width().max(80.0),
                    );
                    let (sum_rect, _) = ui.allocate_exact_size(
                        Vec2::new(
                            ui.available_width().max(galley.size().x),
                            galley.size().y.max(20.0),
                        ),
                        Sense::hover(),
                    );
                    let sum_resp = ui.interact(
                        sum_rect,
                        toggle_id,
                        if ui.is_enabled() {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    sum_resp.widget_info(|| {
                        WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), summary_text)
                    });
                    if sum_resp.has_focus() {
                        ui.painter().rect_stroke(
                            sum_rect.expand(1.0),
                            CornerRadius::same(4),
                            Stroke::new(1.5_f32, pal.accent),
                            StrokeKind::Outside,
                        );
                    }
                    let sum_col = if sum_resp.hovered() {
                        pal.accent
                    } else {
                        pal.text_strong
                    };
                    ui.painter().galley(
                        Pos2::new(sum_rect.left(), sum_rect.center().y - galley.size().y * 0.5),
                        galley,
                        sum_col,
                    );
                    let sum_resp = if ui.is_enabled() {
                        sum_resp.on_hover_cursor(CursorIcon::PointingHand)
                    } else {
                        sum_resp
                    };
                    if sum_resp.clicked() {
                        self.export_help_open = !self.export_help_open;
                    }

                    if self.export_help_open {
                        // Export help remains useful without browser discovery/import.
                        let browsers = [
                            ("chrome", "chromium", "Chrome"),
                            ("edge", "chromium", "Edge"),
                            ("firefox", "firefox", "Firefox"),
                            ("brave", "chromium", "Brave"),
                            ("vivaldi", "chromium", "Vivaldi"),
                            ("opera", "chromium", "Opera"),
                            ("safari", "safari", "Safari"),
                        ];
                        if sources.is_empty() {
                            egui::ComboBox::from_id_salt(panel_id.with("export-browser"))
                                .selected_text(browsers[self.export_browser].2)
                                .show_ui(ui, |ui| {
                                    for (index, (_, _, label)) in browsers.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut self.export_browser,
                                            index,
                                            *label,
                                        );
                                    }
                                });
                        }
                        let (id, family, label) = browsers[self.export_browser];
                        let manual_source = BrowserProfileSource {
                            browser_id: id.into(),
                            family: family.into(),
                            profile_dir: String::new(),
                            label: label.into(),
                        };
                        let selected = self
                            .selected_source
                            .and_then(|idx| sources.get(idx))
                            .or(Some(&manual_source));
                        let profile_display =
                            selected.map_or_else(|| profile_label(french), |s| s.label.as_str());
                        let steps_line = export_steps_for_source(
                            selected.map(|s| s.browser_id.as_str()),
                            selected.map(|s| s.family.as_str()),
                            lang,
                        );

                        ui.add(
                            egui::Label::new(
                                RichText::new(csv_hint_short_label(french))
                                    .size(12.0)
                                    .color(pal.text),
                            )
                            .wrap(),
                        );

                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(
                                RichText::new(format!("1. {}", same_profile_label(french)))
                                    .size(12.0)
                                    .color(pal.text),
                            );
                            ui.label(
                                RichText::new(profile_display)
                                    .size(12.0)
                                    .strong()
                                    .color(pal.text_strong),
                            );
                        });

                        for (num, text) in [
                            (2usize, steps_line),
                            (3usize, save_csv_label(french)),
                            (4usize, return_csv_label(french)),
                            (5usize, remove_csv_label(french)),
                        ] {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{num}. {text}"))
                                        .size(12.0)
                                        .color(pal.text),
                                )
                                .wrap(),
                            );
                        }
                    }
                };
                if input.embedded {
                    let mut content = content;
                    content(ui);
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt(panel_id.with("pw-foot-scroll"))
                        .max_height(180.0)
                        .show(ui, content);
                }
            });
    }
}

struct WideRowColumns {
    site: Rect,
    user: Rect,
    switch: Option<Rect>,
    fill: Rect,
    trash: Rect,
}

fn compute_wide_columns(
    row_rect: Rect,
    fill_btn_w: f32,
    show_agent_switch: bool,
) -> WideRowColumns {
    let pad_x = 16.0;
    let gap = 10.0;
    let trash_x = row_rect.right() - pad_x - ICON_BTN_SIDE;
    let fill_x = trash_x - gap - fill_btn_w;
    let (switch, controls_left) = if show_agent_switch {
        let switch_x = fill_x - gap - SWITCH_WIDTH;
        (
            Some(Rect::from_center_size(
                Pos2::new(switch_x + SWITCH_WIDTH * 0.5, row_rect.center().y),
                Vec2::new(SWITCH_WIDTH, SWITCH_HEIGHT),
            )),
            switch_x,
        )
    } else {
        (None, fill_x)
    };
    let text_avail = (controls_left - gap - (row_rect.left() + pad_x)).max(60.0);
    let site_w = (text_avail - gap).max(40.0) * (1.3 / 2.3);
    let user_w = ((text_avail - gap) - site_w).max(20.0);

    let site_left = row_rect.left() + pad_x;
    let user_left = site_left + site_w + gap;

    WideRowColumns {
        site: Rect::from_min_max(
            Pos2::new(site_left, row_rect.top()),
            Pos2::new(site_left + site_w, row_rect.bottom()),
        ),
        user: Rect::from_min_max(
            Pos2::new(user_left, row_rect.top()),
            Pos2::new(user_left + user_w, row_rect.bottom()),
        ),
        switch,
        fill: Rect::from_center_size(
            Pos2::new(fill_x + fill_btn_w * 0.5, row_rect.center().y),
            Vec2::new(fill_btn_w, SMALL_BTN_HEIGHT),
        ),
        trash: Rect::from_center_size(
            Pos2::new(trash_x + ICON_BTN_SIDE * 0.5, row_rect.center().y),
            Vec2::splat(ICON_BTN_SIDE),
        ),
    }
}

fn render_header(
    ui: &mut Ui,
    panel_id: Id,
    pal: &UiPalette,
    french: bool,
    embedded: bool,
) -> Option<BrowserPasswordsAction> {
    let mut action = None;
    egui::Frame::NONE
        .inner_margin(Margin {
            left: 16,
            right: 16,
            top: 12,
            bottom: 8,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (key_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_local_icon(ui.painter(), key_rect, LocalIcon::KeyRound, pal.text_strong);
                ui.label(
                    RichText::new(dialog_title(french))
                        .size(15.0)
                        .strong()
                        .color(pal.text_strong),
                );
                if embedded {
                    return;
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let close_text = close_label(french);
                    if render_icon_button(
                        ui,
                        panel_id.with("pw-close"),
                        close_text,
                        false,
                        ui.is_enabled(),
                        pal,
                        |painter, rect, col| {
                            icons::paint(painter, rect, Icon::Close, col);
                        },
                        16.0,
                    )
                    .clicked()
                    {
                        action = Some(BrowserPasswordsAction::Close);
                    }
                });
            });
        });
    action
}

fn render_centered_empty_note(ui: &mut Ui, text: &str, pal: &UiPalette) {
    egui::Frame::NONE
        .inner_margin(Margin::symmetric(16, 18))
        .show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(pal.text_dim)).wrap());
            });
        });
}

fn measure_small_button_width(ui: &Ui, label: &str) -> f32 {
    measure_exit_button_width(ui, label, false, true).max(42.0)
}

fn measure_exit_button_width(ui: &Ui, label: &str, has_icon: bool, is_small: bool) -> f32 {
    let font_px = if is_small { 11.5 } else { 12.0 };
    let pad_x = if is_small { 9.0 } else { 12.0 };
    let icon_extra = if has_icon { 19.0 } else { 0.0 };
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        FontId::proportional(font_px),
        Color32::WHITE,
    );
    (galley.size().x + pad_x * 2.0 + icon_extra).ceil()
}

fn render_exit_button(
    ui: &mut Ui,
    id: Id,
    label: &str,
    icon: Option<LocalIcon>,
    style: ExitButtonStyle,
    enabled: bool,
    pal: &UiPalette,
) -> Response {
    let is_small = matches!(style, ExitButtonStyle::Small | ExitButtonStyle::SmallDanger);
    let height = if is_small {
        SMALL_BTN_HEIGHT
    } else {
        CONTROL_HEIGHT
    };
    let width = measure_exit_button_width(ui, label, icon.is_some(), is_small);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    render_exit_button_in_rect(ui, rect, id, label, icon, style, enabled, pal)
}

#[allow(clippy::too_many_arguments)]
fn render_exit_button_in_rect(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    label: &str,
    icon: Option<LocalIcon>,
    style: ExitButtonStyle,
    enabled: bool,
    pal: &UiPalette,
) -> Response {
    let resp = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));

    let hovered = enabled && resp.hovered();
    let dark_mode = ui.visuals().dark_mode;
    let (mut bg, mut border, mut fg) = match style {
        ExitButtonStyle::Primary => {
            let fill = if hovered {
                brighten(pal.accent, 1.08)
            } else {
                pal.accent
            };
            let [r, g, b, _] = fill.to_srgba_unmultiplied();
            let lum = 0.299 * (r as f32) + 0.587 * (g as f32) + 0.114 * (b as f32);
            let primary_fg = if lum > 140.0 {
                PRIMARY_BTN_FG
            } else {
                Color32::WHITE
            };
            (fill, Color32::TRANSPARENT, primary_fg)
        }
        ExitButtonStyle::Standard | ExitButtonStyle::Small => (
            if hovered {
                pal.surface_3
            } else {
                Color32::TRANSPARENT
            },
            pal.border_strong,
            pal.text,
        ),
        ExitButtonStyle::SmallDanger => (
            if hovered {
                with_alpha(pal.danger, 0.25)
            } else {
                Color32::TRANSPARENT
            },
            pal.border_strong,
            if dark_mode {
                DANGER_TEXT_TINT
            } else {
                pal.danger
            },
        ),
    };

    if !enabled {
        bg = with_alpha(bg, 0.5);
        border = with_alpha(border, 0.5);
        fg = with_alpha(fg, 0.5);
    }

    ui.painter().rect(
        rect,
        CornerRadius::same(6),
        bg,
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            CornerRadius::same(6),
            Stroke::new(2.0_f32, pal.accent),
            StrokeKind::Outside,
        );
    }

    let is_small = matches!(style, ExitButtonStyle::Small | ExitButtonStyle::SmallDanger);
    let font_px = if is_small { 11.5 } else { 12.0 };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(font_px), fg);
    if let Some(ic) = icon {
        let total_w = 13.0 + 6.0 + galley.size().x;
        let start_x = (rect.center().x - total_w * 0.5).max(rect.left() + 6.0);
        let ic_rect =
            Rect::from_center_size(Pos2::new(start_x + 6.5, rect.center().y), Vec2::splat(13.0));
        paint_local_icon(ui.painter(), ic_rect, ic, fg);
        ui.painter().galley(
            Pos2::new(start_x + 19.0, rect.center().y - galley.size().y * 0.5),
            galley,
            fg,
        );
    } else {
        ui.painter().galley(
            Pos2::new(
                rect.center().x - galley.size().x * 0.5,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            fg,
        );
    }

    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

#[allow(clippy::too_many_arguments)]
fn render_icon_button(
    ui: &mut Ui,
    id: Id,
    label: &str,
    danger: bool,
    enabled: bool,
    pal: &UiPalette,
    paint_fn: impl FnOnce(&Painter, Rect, Color32),
    icon_px: f32,
) -> Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(ICON_BTN_SIDE), Sense::hover());
    render_icon_button_in_rect(ui, rect, id, label, danger, enabled, pal, paint_fn, icon_px)
}

#[allow(clippy::too_many_arguments)]
fn render_icon_button_in_rect(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    label: &str,
    danger: bool,
    enabled: bool,
    pal: &UiPalette,
    paint_fn: impl FnOnce(&Painter, Rect, Color32),
    icon_px: f32,
) -> Response {
    let resp = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));

    let hovered = enabled && resp.hovered();
    if hovered {
        let hover_bg = if danger {
            with_alpha(pal.danger, 0.25)
        } else {
            pal.surface_3
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), hover_bg);
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            CornerRadius::same(6),
            Stroke::new(2.0_f32, pal.accent),
            StrokeKind::Outside,
        );
    }
    let fg = if !enabled {
        with_alpha(pal.text_dim, 0.5)
    } else if hovered && danger {
        if ui.visuals().dark_mode {
            DANGER_TEXT_TINT
        } else {
            pal.danger
        }
    } else if hovered {
        pal.text_strong
    } else {
        pal.text_dim
    };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icon_px));
    paint_fn(ui.painter(), icon_rect, fg);
    let resp = if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    };
    resp.on_hover_text(label)
}

#[allow(clippy::too_many_arguments)]
fn render_switch_pill(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    label: &str,
    checked: bool,
    indeterminate: bool,
    enabled: bool,
    pal: &UiPalette,
) -> Response {
    let resp = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, checked, label));

    let mut fill = if checked {
        pal.accent
    } else if indeterminate {
        mix_color(pal.accent, pal.surface_3, 0.45)
    } else {
        pal.surface_3
    };
    let mut stroke_col = pal.border_strong;
    let mut thumb_col = if checked || indeterminate {
        Color32::WHITE
    } else {
        Color32::from_rgb(0xae, 0xb4, 0xbf)
    };

    if !enabled {
        fill = with_alpha(fill, 0.5);
        stroke_col = with_alpha(stroke_col, 0.5);
        thumb_col = with_alpha(thumb_col, 0.5);
    }

    ui.painter().rect_filled(rect, CornerRadius::same(10), fill);
    if !checked {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(10),
            Stroke::new(1.0_f32, stroke_col),
            StrokeKind::Inside,
        );
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(12),
            Stroke::new(2.0_f32, pal.accent),
            StrokeKind::Outside,
        );
    }

    if indeterminate && !checked {
        let bar = Rect::from_center_size(rect.center(), Vec2::new(12.0, 3.0));
        ui.painter()
            .rect_filled(bar, CornerRadius::same(2), thumb_col);
    } else {
        let cx = if checked {
            rect.right() - 10.0
        } else {
            rect.left() + 10.0
        };
        ui.painter()
            .circle_filled(Pos2::new(cx, rect.center().y), 7.0, thumb_col);
    }

    let resp = if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    };
    resp.on_hover_text(label)
}

fn paint_truncated_line(ui: &Ui, rect: Rect, text: &str, font: FontId, color: Color32) {
    if !rect.is_positive() || text.is_empty() {
        return;
    }
    let clip = rect.intersect(ui.clip_rect());
    if !clip.is_positive() {
        return;
    }
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat {
            font_id: font,
            color,
            ..Default::default()
        },
    );
    job.wrap = egui::text::TextWrapping {
        max_width: rect.width(),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let pos = Pos2::new(rect.left(), rect.center().y - galley.size().y * 0.5);
    ui.painter().with_clip_rect(clip).galley(pos, galley, color);
}

fn paint_hline(ui: &mut Ui, color: Color32) {
    let w = ui.available_width().max(1.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0_f32, color),
    );
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

/// Paints Lucide `v0.577.0` 24x24 vector icons used by `BrowserPasswords.vue`
/// (`KeyRound`, `Download`, `FileUp`, `Trash2`, `TriangleAlert`, `Check`; covered by
/// `THIRD_PARTY_NOTICES.md` `src/*_panel.rs`).
fn paint_local_icon(painter: &Painter, rect: Rect, icon: LocalIcon, color: Color32) {
    if !rect.is_positive() {
        return;
    }
    let side = rect.width().min(rect.height());
    if side <= 0.0 || !side.is_finite() {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let scale = side / 24.0;
    let pt = |x: f32, y: f32| Pos2::new(sq.min.x + x * scale, sq.min.y + y * scale);
    let stroke = Stroke::new((2.0_f32 * scale).max(1.0_f32), color);

    let polyline = |pts: &[(f32, f32)], closed: bool| {
        let mapped: Vec<Pos2> = pts.iter().map(|&(x, y)| pt(x, y)).collect();
        if closed {
            painter.add(Shape::closed_line(mapped, stroke));
        } else {
            painter.add(Shape::line(mapped, stroke));
        }
    };

    match icon {
        LocalIcon::Check => {
            polyline(&[(20.0, 6.0), (9.0, 17.0), (4.0, 12.0)], false);
        }
        LocalIcon::Download => {
            polyline(&[(12.0, 3.0), (12.0, 15.0)], false);
            polyline(&[(7.0, 10.0), (12.0, 15.0), (17.0, 10.0)], false);
            polyline(
                &[
                    (21.0, 15.0),
                    (21.0, 19.5),
                    (19.5, 21.0),
                    (4.5, 21.0),
                    (3.0, 19.5),
                    (3.0, 15.0),
                ],
                false,
            );
        }
        LocalIcon::FileUp => {
            polyline(
                &[
                    (6.0, 2.0),
                    (14.5, 2.0),
                    (20.0, 7.5),
                    (20.0, 20.0),
                    (18.5, 22.0),
                    (5.5, 22.0),
                    (4.0, 20.0),
                    (4.0, 4.0),
                    (5.5, 2.0),
                ],
                true,
            );
            polyline(&[(14.0, 2.0), (14.0, 7.5), (20.0, 7.5)], false);
            polyline(&[(12.0, 18.0), (12.0, 12.0)], false);
            polyline(&[(9.0, 15.0), (12.0, 12.0), (15.0, 15.0)], false);
        }
        LocalIcon::Trash2 => {
            polyline(&[(3.0, 6.0), (21.0, 6.0)], false);
            polyline(
                &[
                    (8.0, 6.0),
                    (8.0, 3.5),
                    (9.5, 2.0),
                    (14.5, 2.0),
                    (16.0, 3.5),
                    (16.0, 6.0),
                ],
                false,
            );
            polyline(
                &[
                    (5.0, 6.0),
                    (5.0, 20.0),
                    (6.8, 22.0),
                    (17.2, 22.0),
                    (19.0, 20.0),
                    (19.0, 6.0),
                ],
                false,
            );
            polyline(&[(10.0, 11.0), (10.0, 17.0)], false);
            polyline(&[(14.0, 11.0), (14.0, 17.0)], false);
        }
        LocalIcon::TriangleAlert => {
            polyline(
                &[
                    (12.0, 3.3),
                    (21.4, 19.4),
                    (20.2, 21.0),
                    (3.8, 21.0),
                    (2.6, 19.4),
                ],
                true,
            );
            polyline(&[(12.0, 9.0), (12.0, 13.5)], false);
            painter.circle_filled(pt(12.0, 17.2), (1.15 * scale).max(0.9), color);
        }
        LocalIcon::KeyRound => {
            // Circular bow centered at (15.5, 8.5) with radius 6.5 from 108 deg clockwise to 162 deg,
            // joined to stepped bit shaft from (13.4, 14.6) to (2.0, 21.0) and back to (9.4, 10.6).
            let mut pts = Vec::with_capacity(28);
            let start_ang = (14.6_f32 - 8.5).atan2(13.4 - 15.5);
            let mut end_ang = (10.6_f32 - 8.5).atan2(9.4 - 15.5);
            if end_ang >= start_ang {
                end_ang -= std::f32::consts::TAU;
            }
            for step in 0..=14 {
                let t = step as f32 / 14.0;
                let ang = start_ang + (end_ang - start_ang) * t;
                pts.push((15.5 + 6.5 * ang.cos(), 8.5 + 6.5 * ang.sin()));
            }
            pts.extend_from_slice(&[
                (2.4, 17.6),
                (2.0, 18.8),
                (2.0, 21.2),
                (2.8, 22.0),
                (6.2, 22.0),
                (7.0, 21.2),
                (7.0, 19.0),
                (10.0, 19.0),
                (10.0, 16.0),
                (12.0, 16.0),
            ]);
            polyline(&pts, true);
            painter.circle_filled(pt(16.5, 7.5), (1.15 * scale).max(0.9), color);
        }
    }
}

// ============================================================================
// Exact EN/FR localized strings (`BrowserPasswords.vue` + `locales/fr/passwords.json`)
// ============================================================================

const fn dialog_title(french: bool) -> &'static str {
    if french {
        "Mots de passe enregistrés"
    } else {
        "Saved passwords"
    }
}

const fn close_label(french: bool) -> &'static str {
    if french { "Fermer" } else { "Close" }
}

const fn cancel_label(french: bool) -> &'static str {
    if french { "Annuler" } else { "Cancel" }
}

const fn import_all_label(french: bool) -> &'static str {
    if french {
        "Importer de tous les navigateurs"
    } else {
        "Import from all browsers"
    }
}

fn importing_progress_label(prog: ImportProgress, french: bool) -> String {
    if french {
        format!("Import {} sur {}…", prog.done, prog.total)
    } else {
        format!("Importing {} of {}…", prog.done, prog.total)
    }
}

const fn profile_label(french: bool) -> &'static str {
    if french {
        "Profil du navigateur"
    } else {
        "Browser profile"
    }
}

const fn import_one_label(french: bool) -> &'static str {
    if french {
        "Importer ce profil"
    } else {
        "Import this profile"
    }
}

const fn import_csv_label(french: bool) -> &'static str {
    if french {
        "Importer un CSV…"
    } else {
        "Import CSV…"
    }
}

const fn no_sources_label(french: bool) -> &'static str {
    if french {
        "Aucun navigateur avec des mots de passe enregistrés n'a été trouvé sur cet ordinateur. Importez plutôt un export CSV."
    } else {
        "No browser with saved passwords was found on this computer. Import a CSV export instead."
    }
}

const fn search_placeholder(french: bool) -> &'static str {
    if french {
        "Chercher un site ou un nom d'utilisateur"
    } else {
        "Search sites and usernames"
    }
}

const fn agents_hint_label(french: bool) -> &'static str {
    if french {
        "Les agents ne peuvent utiliser un compte qu'après votre autorisation ici, un compte à la fois."
    } else {
        "Agents can use an account only after you allow it here, one account at a time."
    }
}

const fn agents_owner_only_label(french: bool) -> &'static str {
    if french {
        "Agents : désactivés (propriétaire seul)"
    } else {
        "Agents: off (owner fill only)"
    }
}

const fn agents_owner_only_hint_label(french: bool) -> &'static str {
    if french {
        "L'accès des agents au coffre n'est pas encore activé ; vous seul pouvez remplir vos connexions dans votre navigateur privé."
    } else {
        "Agent vault access is not enabled yet; only you can fill logins in your private browser."
    }
}

const fn dismiss_label(french: bool) -> &'static str {
    if french { "Masquer" } else { "Hide" }
}

const fn how_export_label(french: bool) -> &'static str {
    if french {
        "Comment exporter"
    } else {
        "How to export"
    }
}

const fn username_col_header(french: bool) -> &'static str {
    if french {
        "NOM D'UTILISATEUR"
    } else {
        "USERNAME"
    }
}

const fn fill_short_label(french: bool) -> &'static str {
    if french { "Remplir" } else { "Fill" }
}

const fn fill_hint_label(french: bool) -> &'static str {
    if french {
        "Remplir cette connexion sur la page ouverte dans ce navigateur (le formulaire n'est pas envoyé)"
    } else {
        "Fill this login on the page open in this browser (the form is not sent)"
    }
}

const fn empty_vault_label(french: bool) -> &'static str {
    if french {
        "Aucun mot de passe enregistré pour l'instant. Importez-les de vos navigateurs ou d'un export CSV."
    } else {
        "No saved passwords yet. Import them from your browsers or from a CSV export."
    }
}

const fn local_short_label(french: bool) -> &'static str {
    if french {
        "Chiffrés sur cet ordinateur, hors de vos projets, jamais transmis par Live Share. L'accès des agents est désactivé pour chaque compte jusqu'à ce que vous l'autorisiez."
    } else {
        "Encrypted on this computer, outside your projects, never sent through Live Share. Agent access is off for every account until you allow it."
    }
}

const fn local_owner_only_label(french: bool) -> &'static str {
    if french {
        "Chiffrés sur cet ordinateur, hors de vos projets, jamais transmis par Live Share. Vous seul pouvez remplir ces connexions — l'accès des agents au coffre est désactivé."
    } else {
        "Encrypted on this computer, outside your projects, never sent through Live Share. Only you can fill these logins — agent vault access is disabled."
    }
}

const fn export_help_summary(french: bool) -> &'static str {
    if french {
        "Comment exporter les mots de passe de ce navigateur"
    } else {
        "How to export passwords for this browser"
    }
}

const fn csv_hint_short_label(french: bool) -> &'static str {
    if french {
        "Quand un navigateur bloque l'import, exportez ses mots de passe dans un fichier CSV et importez ce fichier. Les clés d'accès et les codes de vérification ne sont pas importés."
    } else {
        "When a browser blocks the import, export its passwords as a CSV file and import that file. Passkeys and verification codes are not imported."
    }
}

const fn same_profile_label(french: bool) -> &'static str {
    if french {
        "Ouvrez le navigateur d'origine (pas Tessel) et sélectionnez ce même profil :"
    } else {
        "Open the original browser (not Tessel) and select this same profile:"
    }
}

const fn save_csv_label(french: bool) -> &'static str {
    if french {
        "Confirmez avec le mot de passe de votre appareil ou Windows Hello si demandé, puis enregistrez le CSV sur cet ordinateur, hors d'un projet partagé."
    } else {
        "Confirm with your device password or Windows Hello if requested, then save the CSV on this computer, outside any shared project."
    }
}

const fn return_csv_label(french: bool) -> &'static str {
    if french {
        "Revenez ici, cliquez sur Importer un CSV et sélectionnez le fichier enregistré."
    } else {
        "Return here, click Import CSV, and select the file you just saved."
    }
}

const fn remove_csv_label(french: bool) -> &'static str {
    if french {
        "Après avoir confirmé l'import, supprimez le CSV lorsqu'il n'est plus nécessaire : il contient les mots de passe en clair. Ne l'envoyez pas dans le chat ni par Live Share."
    } else {
        "After confirming the import, delete the CSV when no longer needed: it contains readable passwords. Do not send it in chat or through Live Share."
    }
}
