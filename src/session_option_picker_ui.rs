// Adapted from Tessel SessionOptionPicker.vue, TerminalPane.vue, PaneActionsMenu.vue,
// sessionOptionLabels.js, and Orca (github.com/stablyai/orca, MIT, Copyright (c) 2026
// Lovecast Inc.; see THIRD_PARTY_NOTICES.md).
//! Native `egui` per-pane model & session-option picker (`SessionOptionPicker.vue`),
//! pane header model chip (`.pane-model-chip`), `"Restart to apply"` header badge
//! (`.pane-apply`), `PaneActionsMenu` model entry resolver, and capability-gated
//! declarative action planner bridged to [`crate::pane_session_model`].
//!
//! # Scope & Non-Execution Contract
//! - Pure UI rendering and declarative output only: never writes to a PTY, spawns a
//!   process, touches disk, or mutates global state.
//! - Live mid-session commands (`/model`, `/effort`, `/fast`, `"Choose in agent picker…"`)
//!   and the `"Restart to apply"` badge are gated behind explicit
//!   [`SessionPickerCapabilities`] flags (all `false` by default) so unwired host
//!   commands are never advertised.

use crate::{
    agent_model_catalog as catalog,
    agent_model_defaults::Defaults,
    agent_model_defaults_ui::{
        is_launch_option, model_description, model_field_label, option_default_label,
        session_choice_label, session_option_label, session_pill_label, values_on_listed_row,
    },
    pane_session_model::{
        ClaudeSwitchOutcome, ModelPickOutcome, ModelPickPlan, PaneModelNode, PanePickContext,
        PlannedToast, ShownAgentModel, ToastKind, apply_restart_title, chip_aria_label,
        effective_model_id, header_model_text, launch_session_values, model_busy_reason,
        model_default_label, model_label, model_menu_note, model_text, model_title,
        pane_menu_values, plan_model_action, plan_model_pick, restart_apply_badge_text,
    },
    theme,
};
use eframe::egui::{
    self, Align, Color32, CornerRadius, CursorIcon, EventFilter, FontId, Id, Key, Margin,
    Modifiers, Order, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
    WidgetType,
};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// `@container (max-width: 340px) { .pane-model-chip { display: none; } }` (`style.css:742`).
pub const HEADER_MODEL_CHIP_HIDE_MAX_WIDTH: f32 = 340.0;
/// `.pane-model-chip { max-width: 140px; }` (`style.css:715`).
pub const HEADER_MODEL_CHIP_MAX_WIDTH: f32 = 140.0;
/// `.pane-model-chip` / `.pane-apply` height (`style.css:716, 10233`).
pub const HEADER_CHIP_HEIGHT: f32 = 18.0;
/// `.sop { min-width: 220px; }` (`SessionOptionPicker.vue:223`).
pub const PICKER_MIN_WIDTH: f32 = 220.0;
/// `.sop { max-width: 300px; }` (`SessionOptionPicker.vue:224`).
pub const PICKER_MAX_WIDTH: f32 = 300.0;
/// `.sop { max-height: min(70vh, 520px); }` (`SessionOptionPicker.vue:225`).
pub const PICKER_MAX_HEIGHT: f32 = 520.0;
/// Viewport clamping margin for `.pane-model-menu` (`TerminalPane.vue:752-755`).
pub const PICKER_VIEWPORT_MARGIN: f32 = 4.0;
/// Vertical gap below `.pane-model-chip` when opening the picker (`TerminalPane.vue:734`).
pub const CHIP_ANCHOR_OFFSET_Y: f32 = 2.0;

const POPOVER_OUTER_PADDING: f32 = 4.0;
const POPOVER_OUTER_PADDING_I8: i8 = 4;
const SOP_VERTICAL_PAD: f32 = 2.0;
const SOP_ROW_PAD_X: f32 = 10.0;
const SOP_ROW_PAD_Y: f32 = 4.0;
const SOP_CHECK_WIDTH: f32 = 10.0;
const SOP_CHECK_GAP: f32 = 6.0;
const SOP_LABEL_HEIGHT: f32 = 22.0;
const SOP_SIMPLE_ROW_HEIGHT: f32 = 24.0;
const SOP_SWITCH_ROW_HEIGHT: f32 = 28.0;
const SOP_SEP_HEIGHT: f32 = 9.0;
const SWITCH_WIDTH: f32 = 34.0;
const SWITCH_HEIGHT: f32 = 20.0;
const DISABLED_ALPHA_FACTOR: f32 = 0.55;

/// Explicit runtime capabilities gating live mid-session commands and header actions.
///
/// Defaults to all `false` so unintegrated surfaces never advertise `/model`, `/effort`,
/// `/fast`, `"Choose in agent picker…"`, or `"Restart to apply"` before host PTY/restart
/// plumbing is connected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionPickerCapabilities {
    /// Whether clicking the header model chip or `Model…` menu item opens the picker.
    pub can_open_picker: bool,
    /// Whether live mid-session `/model <val>`, `/effort <val>`, and Cursor `/model <filter>`
    /// commands are wired for running panes.
    pub live_command_switch: bool,
    /// Whether live flip-only toggle commands (e.g. Claude `/fast`) are wired.
    pub live_toggle_command: bool,
    /// Whether live TUI picker commands (`"Choose in agent picker…"`: Codex/Gemini/Pi `/model`,
    /// OpenCode `/models`, Pi `/thinking`) are wired.
    pub live_agent_picker: bool,
    /// Whether the `"Restart to apply"` / `"Redémarrer pour appliquer"` header badge is wired.
    pub can_restart_to_apply: bool,
}

impl SessionPickerCapabilities {
    /// Enables saved-choice picking and restart-to-apply while keeping all live PTY commands off.
    #[must_use]
    pub const fn restart_only() -> Self {
        Self {
            can_open_picker: true,
            live_command_switch: false,
            live_toggle_command: false,
            live_agent_picker: false,
            can_restart_to_apply: true,
        }
    }

    /// Enables full Vue-parity picker opening, live PTY commands, and restart-to-apply.
    #[must_use]
    pub const fn all_live() -> Self {
        Self {
            can_open_picker: true,
            live_command_switch: true,
            live_toggle_command: true,
            live_agent_picker: true,
            can_restart_to_apply: true,
        }
    }
}

/// Returns `true` when `agent_id` exposes per-pane model choices (`TerminalPane.vue:639`).
#[must_use]
pub fn has_model_choice(is_agent: bool, detected: bool, agent_id: &str) -> bool {
    is_agent && !detected && catalog::find(agent_id).is_some()
}

/// Localized `"Choose in agent picker…"` action label (`pane.sessionOptions.chooseInAgentPicker`).
#[must_use]
pub const fn choose_in_agent_picker_label(french: bool) -> &'static str {
    if french {
        "Choisir dans le sélecteur d'agents…"
    } else {
        "Choose in agent picker…"
    }
}

/// Localized `"Toggle {{option}}"` action label (`SessionOptionPicker.vue:87-89`).
#[must_use]
pub fn toggle_option_label(option: &Value, french: bool) -> String {
    let lower = session_option_label(option, french).to_lowercase();
    if french {
        format!("Basculer {lower}")
    } else {
        format!("Toggle {lower}")
    }
}

/// Localized `PaneActionsMenu` interactive item label (`pane.menu.model`).
#[must_use]
pub const fn pane_menu_model_label(french: bool) -> &'static str {
    if french { "Modèle…" } else { "Model…" }
}

/// Localized `PaneActionsMenu` interactive item tooltip (`pane.menu.modelHint`).
#[must_use]
pub const fn pane_menu_model_hint(french: bool) -> &'static str {
    if french {
        "Choisir le modèle utilisé par cet agent"
    } else {
        "Choose the model this agent uses"
    }
}

/// Localized reason shown when a line is currently typed in the running terminal
/// (`pane.chatView.typedLine`).
#[must_use]
pub const fn typed_line_reason(french: bool) -> &'static str {
    if french {
        "Une ligne est tapée dans son terminal : envoyez-la ou effacez-la d'abord."
    } else {
        "A line is typed in its terminal: send or clear it there first."
    }
}

/// Capability-honest context note under `.sop-label` (`TerminalPane.vue:modelMenuNote`).
///
/// When `pane_running` is `true` but `capabilities.live_command_switch` is `false`, never
/// claims immediate live switching (`cursorLive` or empty note); instead reports that picks
/// apply when the agent restarts.
#[must_use]
pub fn capability_gated_menu_note(
    agent_id: &str,
    pane_running: bool,
    capabilities: SessionPickerCapabilities,
    french: bool,
) -> &'static str {
    if !pane_running {
        return model_menu_note(agent_id, false, french);
    }
    if !capabilities.live_command_switch {
        return model_menu_note("codex", true, french);
    }
    model_menu_note(agent_id, true, french)
}

/// How an option in `model.options` renders inside [`SessionOptionPickerUi`]
/// (`SessionOptionPicker.vue:73-84`), gated by [`SessionPickerCapabilities`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionRowMode {
    /// Flip-only action button (`"Toggle fast mode"` / `"Basculer mode rapide"`).
    Toggle,
    /// Section header + `"Choose in agent picker…"` action button.
    AgentPicker,
    /// Inline boolean `.set-switch` row (e.g. Cursor `Fast mode` / `Thinking`).
    Switch,
    /// Section header + `"Default"` radio row + choice radio rows.
    Select,
    /// Omitted (e.g. live-only flip option when stopped or when `live_toggle_command` is unwired).
    Hidden,
}

/// Resolves the [`OptionRowMode`] for `option` under `(pane_running, capabilities)`.
#[must_use]
pub fn option_row_mode(
    agent_id: &str,
    option: &Value,
    pane_running: bool,
    capabilities: SessionPickerCapabilities,
) -> OptionRowMode {
    let mid_kind = option["apply"]["midSession"]["kind"].as_str();
    if pane_running && capabilities.live_toggle_command && mid_kind == Some("toggle-command") {
        return OptionRowMode::Toggle;
    }
    if pane_running && capabilities.live_agent_picker && mid_kind == Some("agent-picker") {
        return OptionRowMode::AgentPicker;
    }
    match option["kind"]["type"].as_str() {
        Some("boolean") => {
            if !is_launch_option(agent_id, option) && option["apply"]["launchArgs"].is_null() {
                OptionRowMode::Hidden
            } else {
                OptionRowMode::Switch
            }
        }
        Some("select") => OptionRowMode::Select,
        _ => OptionRowMode::Hidden,
    }
}

// -----------------------------------------------------------------------------
// 1. Header Model Chip & Restart-to-Apply Badge (`TerminalPane.vue:4402-4455`)
// -----------------------------------------------------------------------------

/// Input for [`show_header_model_chip`].
pub struct HeaderModelChipInput<'a> {
    /// Full inline width of `.pane-nav` (used for the `<= 340px` responsive hide rule).
    pub pane_header_width: f32,
    pub is_agent: bool,
    pub detected: bool,
    pub agent_id: &'a str,
    pub guest_runs: bool,
    pub guest_model: Option<&'a str>,
    pub agent_model: Option<&'a ShownAgentModel>,
    pub capabilities: SessionPickerCapabilities,
    pub french: bool,
}

/// Output from [`show_header_model_chip`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeaderModelChipOutput {
    pub visible: bool,
    pub interactive: bool,
    pub clicked: bool,
    pub text: String,
    pub aria_label: String,
    pub tooltip: String,
    pub rect: Option<Rect>,
    pub id: Option<Id>,
}

/// Renders `.pane-model-chip` (interactive `<button>` or `.static` `<span>`) inside a pane header.
pub fn show_header_model_chip(
    ui: &mut Ui,
    id_source: Id,
    input: &HeaderModelChipInput<'_>,
) -> HeaderModelChipOutput {
    let text = header_model_text(
        input.is_agent,
        input.guest_runs,
        input.guest_model,
        input.agent_model,
        input.french,
    );
    if text.is_empty() || input.pane_header_width <= HEADER_MODEL_CHIP_HIDE_MAX_WIDTH {
        return HeaderModelChipOutput::default();
    }

    let interactive = has_model_choice(input.is_agent, input.detected, input.agent_id)
        && !input.guest_runs
        && input.capabilities.can_open_picker;
    let aria_label = chip_aria_label(&text, interactive, input.guest_runs, input.french);
    let tooltip = if input.guest_runs {
        String::new()
    } else {
        model_title(input.agent_model, input.french)
    };

    let font = FontId::proportional(10.5);
    let text_w = measure_text_width(ui, &text, font.clone());
    let max_w = HEADER_MODEL_CHIP_MAX_WIDTH.min(ui.available_width().max(24.0));
    let chip_w = (text_w + 12.0).ceil().clamp(24.0_f32.min(max_w), max_w);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(chip_w, HEADER_CHIP_HEIGHT), Sense::hover());

    let chip_id = id_source.with("pane-model-chip");
    let enabled = ui.is_enabled();
    let sense = if interactive && enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, chip_id, sense);
    let widget_type = if interactive {
        WidgetType::Button
    } else {
        WidgetType::Label
    };
    let a11y = aria_label.clone();
    response.widget_info(move || WidgetInfo::labeled(widget_type, enabled, &a11y));

    let colors = resolve_colors(ui);
    let active_visual = interactive && enabled && (response.hovered() || response.has_focus());
    let fill = Color32::from_rgba_unmultiplied(
        colors.text_dim.r(),
        colors.text_dim.g(),
        colors.text_dim.b(),
        31,
    );
    ui.painter().rect_filled(rect, CornerRadius::same(9), fill);
    if active_visual {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(9),
            Stroke::new(1.0_f32, colors.border_strong),
            StrokeKind::Inside,
        );
    }
    if interactive && response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                chip_id,
                EventFilter {
                    escape: true,
                    ..Default::default()
                },
            );
        });
        ui.painter().rect_stroke(
            rect.expand(1.5),
            CornerRadius::same(10),
            Stroke::new(1.5_f32, colors.accent),
            StrokeKind::Inside,
        );
    }

    let fg = if active_visual {
        colors.text
    } else {
        colors.text_dim
    };
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + 6.0, rect.top()),
        Pos2::new((rect.right() - 6.0).max(rect.left() + 6.0), rect.bottom()),
    );
    paint_single_line(ui, text_rect, &text, font, fg);

    let response = if interactive && enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    let clicked = interactive && enabled && response.clicked();
    if clicked {
        response.request_focus();
    }
    if !tooltip.is_empty() {
        let _ = response.on_hover_text(&tooltip);
    }

    HeaderModelChipOutput {
        visible: true,
        interactive,
        clicked,
        text,
        aria_label,
        tooltip,
        rect: Some(rect),
        id: Some(chip_id),
    }
}

/// Input for [`show_restart_apply_badge`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RestartApplyBadgeInput {
    pub launch_stale: bool,
    pub capabilities: SessionPickerCapabilities,
    pub french: bool,
}

/// Output from [`show_restart_apply_badge`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RestartApplyBadgeOutput {
    pub visible: bool,
    pub clicked: bool,
    pub text: String,
    pub tooltip: String,
    pub rect: Option<Rect>,
    pub id: Option<Id>,
}

/// Renders `.pane-apply` (`"Restart to apply"` / `"Redémarrer pour appliquer"`) when
/// `launch_stale && capabilities.can_restart_to_apply`.
pub fn show_restart_apply_badge(
    ui: &mut Ui,
    id_source: Id,
    input: RestartApplyBadgeInput,
) -> RestartApplyBadgeOutput {
    if !input.launch_stale || !input.capabilities.can_restart_to_apply {
        return RestartApplyBadgeOutput::default();
    }

    let text = restart_apply_badge_text(input.french).to_owned();
    let tooltip = apply_restart_title(input.french).to_owned();
    let font = FontId::proportional(10.5);
    let text_w = measure_text_width(ui, &text, font.clone());
    let max_w = ui.available_width().max(32.0);
    let badge_w = (text_w + 16.0).ceil().min(max_w);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(badge_w, HEADER_CHIP_HEIGHT), Sense::hover());

    let badge_id = id_source.with("pane-restart-apply");
    let enabled = ui.is_enabled();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, badge_id, sense);
    let a11y = text.clone();
    response.widget_info(move || WidgetInfo::labeled(WidgetType::Button, enabled, &a11y));

    let colors = resolve_colors(ui);
    let highlighted = enabled && (response.hovered() || response.has_focus());
    let fill_alpha = if highlighted { 71 } else { 41 }; // 28% hover vs 16% rest
    let fill = Color32::from_rgba_unmultiplied(
        colors.accent.r(),
        colors.accent.g(),
        colors.accent.b(),
        fill_alpha,
    );
    let border = Color32::from_rgba_unmultiplied(
        colors.accent.r(),
        colors.accent.g(),
        colors.accent.b(),
        140,
    );
    ui.painter().rect_filled(rect, CornerRadius::same(9), fill);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(9),
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    if response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                badge_id,
                EventFilter {
                    escape: true,
                    ..Default::default()
                },
            );
        });
        ui.painter().rect_stroke(
            rect.expand(1.5),
            CornerRadius::same(10),
            Stroke::new(1.5_f32, colors.accent),
            StrokeKind::Inside,
        );
    }

    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + 8.0, rect.top()),
        Pos2::new((rect.right() - 8.0).max(rect.left() + 8.0), rect.bottom()),
    );
    paint_single_line(ui, text_rect, &text, font, colors.text_strong);

    let response = if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    let clicked = enabled && response.clicked();
    if clicked {
        response.request_focus();
    }
    let _ = response.on_hover_text(&tooltip);

    RestartApplyBadgeOutput {
        visible: true,
        clicked,
        text,
        tooltip,
        rect: Some(rect),
        id: Some(badge_id),
    }
}

// -----------------------------------------------------------------------------
// 2. `PaneActionsMenu.vue` Model Entry Resolver (`PaneActionsMenu.vue:86-98`)
// -----------------------------------------------------------------------------

/// Input for [`resolve_pane_menu_model_entry`].
pub struct PaneMenuModelInput<'a> {
    pub is_agent: bool,
    pub detected: bool,
    pub agent_id: &'a str,
    pub session_options: Option<&'a Value>,
    pub discovered_models: Option<&'a [Value]>,
    pub agent_model: Option<&'a ShownAgentModel>,
    pub persisted_defaults: &'a BTreeMap<String, Defaults>,
    pub capabilities: SessionPickerCapabilities,
    pub french: bool,
}

/// Model representation inside a pane's `…` context menu (`PaneActionsMenu.vue:86-98`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneMenuModelEntry {
    /// Non-agent pane or agent with neither picker capability nor known model.
    None,
    /// Interactive `Model…` / `Modèle…` button opening [`SessionOptionPickerUi`].
    InteractiveItem {
        label: String,
        shortcut: String,
        tooltip: String,
        /// `true` when `node.session_options` is set (`.ctx-menu-model.manual` italic styling).
        is_manual_override: bool,
    },
    /// Read-only `Model` / `Modèle` fact row when `has_model_choice` or `can_open_picker` is `false`.
    ReadOnlyFact {
        label: String,
        value: String,
        tooltip: String,
    },
}

/// Resolves the `PaneActionsMenu.vue` model item or read-only fact row.
#[must_use]
pub fn resolve_pane_menu_model_entry(input: &PaneMenuModelInput<'_>) -> PaneMenuModelEntry {
    let can_pick = has_model_choice(input.is_agent, input.detected, input.agent_id)
        && input.capabilities.can_open_picker;
    let shown = model_text(input.agent_model, input.french);
    if can_pick {
        let models = catalog::models_for(input.agent_id, input.discovered_models);
        let pane_vals = launch_session_values(
            input.session_options,
            input.persisted_defaults,
            input.agent_id,
        );
        let shortcut = if shown.is_empty() {
            session_pill_label(&models, pane_vals.as_ref(), input.french)
        } else {
            shown
        };
        return PaneMenuModelEntry::InteractiveItem {
            label: pane_menu_model_label(input.french).to_owned(),
            shortcut,
            tooltip: pane_menu_model_hint(input.french).to_owned(),
            is_manual_override: input.session_options.is_some(),
        };
    }
    if input.is_agent && !shown.is_empty() {
        return PaneMenuModelEntry::ReadOnlyFact {
            label: model_field_label(input.french).to_owned(),
            value: shown,
            tooltip: model_title(input.agent_model, input.french),
        };
    }
    PaneMenuModelEntry::None
}

// -----------------------------------------------------------------------------
// 3. `SessionOptionPickerUi` (`SessionOptionPicker.vue` + `.pane-model-menu`)
// -----------------------------------------------------------------------------

/// Declarative intent emitted by [`SessionOptionPickerUi`] (`emit('set' | 'action')`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionPickerEvent {
    /// Emitted when a model radio row, option radio row, or boolean switch is chosen.
    /// `value: None` resets `"model"` or a `"select"` option back to its default.
    Set {
        option_id: String,
        value: Option<Value>,
    },
    /// Emitted when `"Choose in agent picker…"` or `"Toggle fast mode"` is clicked.
    Action { option_id: String },
}

/// Reason an open [`SessionOptionPickerUi`] popover closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerCloseReason {
    /// Closed via `Escape` (`closeModelMenu(true)` -> refocus terminal).
    Escape,
    /// Closed by clicking outside the popover (`closeModelMenu(false)`).
    OutsideClick,
    /// Closed by clicking the anchor chip again while open.
    TriggerToggle,
    /// Closed after a live switch/action (`closeModelMenu(true)` -> refocus terminal).
    ActionCompleted,
}

impl PickerCloseReason {
    /// Whether the host should return keyboard focus to the pane's terminal (`closeModelMenu(refocus)`).
    #[must_use]
    pub const fn refocus_pane(self) -> bool {
        matches!(self, Self::Escape | Self::ActionCompleted)
    }
}

/// Frame input for [`SessionOptionPickerUi::show_inline`] and [`SessionOptionPickerUi::show_popover`].
pub struct SessionPickerInput<'a> {
    pub agent_id: &'a str,
    pub is_agent: bool,
    pub detected: bool,
    pub pane_running: bool,
    pub pane_busy: bool,
    pub pending: bool,
    /// Guards live terminal command rows while the user has uncommitted text on the prompt line.
    pub user_typing: bool,
    pub session_options: Option<&'a Value>,
    pub discovered_models: Option<&'a [Value]>,
    pub agent_model: Option<&'a ShownAgentModel>,
    pub persisted_defaults: &'a BTreeMap<String, Defaults>,
    pub capabilities: SessionPickerCapabilities,
    pub french: bool,
    /// Renders `sop-model-default` (`"Default from Settings (...)"` / `"Agent's own default"`).
    pub show_default_row: bool,
    /// Optional subtitle under `sop-model-default` (`SessionOptionPicker.vue:defaultHint`).
    pub default_hint: Option<&'a str>,
    /// Optional explicit override for `disabledReason`.
    pub disabled_reason_override: Option<&'a str>,
    /// Optional trigger button rect so outside-click detection ignores the trigger itself.
    pub trigger_rect: Option<Rect>,
}

impl<'a> SessionPickerInput<'a> {
    /// Constructs a standard pane-menu picker input for `agent_id`.
    #[must_use]
    pub fn new(agent_id: &'a str, persisted_defaults: &'a BTreeMap<String, Defaults>) -> Self {
        Self {
            agent_id,
            is_agent: true,
            detected: false,
            pane_running: false,
            pane_busy: false,
            pending: false,
            user_typing: false,
            session_options: None,
            discovered_models: None,
            agent_model: None,
            persisted_defaults,
            capabilities: SessionPickerCapabilities::default(),
            french: false,
            show_default_row: true,
            default_hint: None,
            disabled_reason_override: None,
            trigger_rect: None,
        }
    }
}

/// Frame output from [`SessionOptionPickerUi::show_inline`] and [`SessionOptionPickerUi::show_popover`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionPickerOutput {
    pub supported: bool,
    pub disabled: bool,
    pub chosen_model: Option<String>,
    pub options_model: Option<String>,
    pub note: String,
    pub disabled_reason: String,
    pub event: Option<SessionPickerEvent>,
    pub closed: Option<PickerCloseReason>,
    pub escape_consumed: bool,
    pub popup_rect: Option<Rect>,
    pub focus_ids: Vec<Id>,
}

#[derive(Clone, Debug)]
struct OpenPopoverState {
    anchor_pos: Pos2,
    trigger_rect: Option<Rect>,
    return_focus_id: Option<Id>,
    opened_pass: u64,
    needs_initial_focus: bool,
}

/// Stateful per-pane session model & option picker (`SessionOptionPicker.vue` + `.pane-model-menu`).
#[derive(Clone, Debug, Default)]
pub struct SessionOptionPickerUi {
    popover: Option<OpenPopoverState>,
    last_popup_rect: Option<Rect>,
    active_index: Option<usize>,
    scroll_to_active: bool,
    last_agent_id: Option<String>,
    escape_held: bool,
    escape_return_focus: Option<Id>,
    escape_consumed_pass: Option<u64>,
    pending_close: Option<PickerCloseReason>,
}

impl SessionOptionPickerUi {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets all transient popover, focus, and key-repeat state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Returns `true` when the floating `.pane-model-menu` popover is currently open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.popover.is_some()
    }

    /// Requested anchor position before viewport clamping, if open.
    #[must_use]
    pub fn anchor_pos(&self) -> Option<Pos2> {
        self.popover.as_ref().map(|p| p.anchor_pos)
    }

    /// Most recently painted clamped `.pane-model-menu` rectangle, if open.
    #[must_use]
    pub fn last_popup_rect(&self) -> Option<Rect> {
        self.last_popup_rect
    }

    /// Opens `.pane-model-menu` at `pos` (`TerminalPane.vue:openModelMenuAt`).
    pub fn open_at_point(&mut self, ctx: &egui::Context, pos: Pos2, return_focus_id: Option<Id>) {
        self.popover = Some(OpenPopoverState {
            anchor_pos: pos,
            trigger_rect: None,
            return_focus_id,
            opened_pass: ctx.cumulative_pass_nr(),
            needs_initial_focus: true,
        });
        self.active_index = None;
        self.scroll_to_active = true;
        self.pending_close = None;
    }

    /// Opens `.pane-model-menu` anchored under `chip_rect` (`(chip.left, chip.bottom + 2)`,
    /// `TerminalPane.vue:openModelMenuAtChip`).
    pub fn open_at_chip(
        &mut self,
        ctx: &egui::Context,
        chip_rect: Rect,
        return_focus_id: Option<Id>,
    ) {
        self.popover = Some(OpenPopoverState {
            anchor_pos: Pos2::new(chip_rect.left(), chip_rect.bottom() + CHIP_ANCHOR_OFFSET_Y),
            trigger_rect: Some(chip_rect),
            return_focus_id,
            opened_pass: ctx.cumulative_pass_nr(),
            needs_initial_focus: true,
        });
        self.active_index = None;
        self.scroll_to_active = true;
        self.pending_close = None;
    }

    /// Toggles `.pane-model-menu` from `chip_rect`.
    pub fn toggle_at_chip(
        &mut self,
        ctx: &egui::Context,
        chip_rect: Rect,
        return_focus_id: Option<Id>,
    ) {
        if self.is_open() {
            self.close_with_reason(ctx, PickerCloseReason::TriggerToggle);
        } else {
            self.open_at_chip(ctx, chip_rect, return_focus_id);
        }
    }

    /// Closes the popover without recording a close event.
    pub fn close(&mut self) {
        self.popover = None;
        self.last_popup_rect = None;
        self.active_index = None;
    }

    /// Closes the popover, records `reason` for the current frame output, and restores focus if needed.
    pub fn close_with_reason(&mut self, ctx: &egui::Context, reason: PickerCloseReason) {
        if let Some(open) = self.popover.take()
            && let Some(ret_id) = open.return_focus_id
            && reason == PickerCloseReason::Escape
        {
            ctx.memory_mut(|m| m.request_focus(ret_id));
        }
        self.last_popup_rect = None;
        self.active_index = None;
        self.pending_close = Some(reason);
    }

    /// Intercepts `Escape` while the popover is open so parent modals/menus do not close on the same press,
    /// absorbing key-repeat frames until `Escape` is released.
    pub fn handle_escape(&mut self, ctx: &egui::Context) -> bool {
        let down = ctx.input(|i| i.key_down(Key::Escape));
        if !down {
            self.escape_held = false;
            self.escape_return_focus = None;
        }
        if !ctx.input(|i| i.focused) {
            return false;
        }
        if self.popover.is_none() && !self.escape_held {
            return false;
        }
        let consumed = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if !(consumed || down) {
            return false;
        }
        if let Some(open) = self.popover.take() {
            self.escape_return_focus = open.return_focus_id;
            if let Some(ret_id) = open.return_focus_id {
                ctx.memory_mut(|m| m.request_focus(ret_id));
            }
            self.last_popup_rect = None;
            self.active_index = None;
            self.escape_held = down;
            self.escape_consumed_pass = Some(ctx.cumulative_pass_nr());
            self.pending_close = Some(PickerCloseReason::Escape);
            return true;
        }
        if consumed && self.escape_held {
            // egui clears keyboard focus before dispatching each Escape repeat.
            // Keep the original trigger focused until this press is released.
            if let Some(ret_id) = self.escape_return_focus {
                ctx.memory_mut(|m| m.request_focus(ret_id));
            }
            self.escape_consumed_pass = Some(ctx.cumulative_pass_nr());
            return true;
        }
        false
    }

    /// Renders `.sop` inline inside `ui` (`min-width: 220px`, `max-width: 300px`).
    pub fn show_inline(
        &mut self,
        ui: &mut Ui,
        input: &SessionPickerInput<'_>,
    ) -> SessionPickerOutput {
        let Some(resolved) = resolve_picker_view(input) else {
            return SessionPickerOutput::default();
        };
        if self.last_agent_id.as_deref() != Some(input.agent_id) {
            self.active_index = None;
            self.last_agent_id = Some(input.agent_id.to_owned());
        }

        let avail_w = ui.available_width().max(60.0);
        let natural_w = measure_sop_natural_width(ui.ctx(), &resolved);
        let sop_w = natural_w.clamp(PICKER_MIN_WIDTH.min(avail_w), PICKER_MAX_WIDTH.min(avail_w));
        let max_h = (ui.ctx().content_rect().height() * 0.70).clamp(80.0, PICKER_MAX_HEIGHT);

        let parent_enabled = ui.is_enabled();
        let id_base = ui.make_persistent_id(("session-option-picker", input.agent_id));
        let (event, focus_ids) = self.render_sop_contents(
            ui,
            SopRenderParams {
                id_base,
                width: sop_w,
                max_height: max_h,
                request_initial_focus: false,
                resolved: &resolved,
            },
        );

        SessionPickerOutput {
            supported: true,
            disabled: resolved.disabled || !parent_enabled,
            chosen_model: resolved.chosen_model,
            options_model: resolved.options_model,
            note: resolved.note,
            disabled_reason: resolved.disabled_reason,
            event,
            closed: self.pending_close.take(),
            escape_consumed: self.escape_consumed_pass == Some(ui.ctx().cumulative_pass_nr()),
            popup_rect: None,
            focus_ids,
        }
    }

    /// Renders `.ctx-menu.pane-model-menu` as a viewport-clamped floating popover when open.
    pub fn show_popover(
        &mut self,
        ctx: &egui::Context,
        id_source: Id,
        input: &SessionPickerInput<'_>,
    ) -> SessionPickerOutput {
        let pass_nr = ctx.cumulative_pass_nr();
        let mut escape_consumed = self.escape_consumed_pass == Some(pass_nr);
        if !escape_consumed && self.handle_escape(ctx) {
            escape_consumed = true;
        }

        let Some(resolved) = resolve_picker_view(input) else {
            self.close();
            return SessionPickerOutput {
                closed: self.pending_close.take(),
                escape_consumed,
                ..Default::default()
            };
        };

        if !input.capabilities.can_open_picker {
            self.close();
            return SessionPickerOutput {
                supported: true,
                disabled: resolved.disabled,
                chosen_model: resolved.chosen_model,
                options_model: resolved.options_model,
                note: resolved.note,
                disabled_reason: resolved.disabled_reason,
                closed: self.pending_close.take(),
                escape_consumed,
                ..Default::default()
            };
        }

        let Some(open_state) = self.popover.clone() else {
            return SessionPickerOutput {
                supported: true,
                disabled: resolved.disabled,
                chosen_model: resolved.chosen_model,
                options_model: resolved.options_model,
                note: resolved.note,
                disabled_reason: resolved.disabled_reason,
                closed: self.pending_close.take(),
                escape_consumed,
                ..Default::default()
            };
        };

        if self.last_agent_id.as_deref() != Some(input.agent_id) {
            self.active_index = None;
            self.last_agent_id = Some(input.agent_id.to_owned());
        }

        let (popup_rect, inner_w, max_sop_h) =
            compute_clamped_popover_rect(ctx, open_state.anchor_pos, &resolved);

        if open_state.opened_pass != pass_nr {
            let ignored_trigger = input.trigger_rect.or(open_state.trigger_rect);
            let outside_pressed = ctx.input(|i| {
                (i.pointer.button_pressed(PointerButton::Primary)
                    || i.pointer.button_pressed(PointerButton::Secondary))
                    && i.pointer.interact_pos().is_some_and(|pos| {
                        !popup_rect.contains(pos)
                            && !ignored_trigger.is_some_and(|tr| tr.contains(pos))
                    })
            });
            if outside_pressed {
                self.close_with_reason(ctx, PickerCloseReason::OutsideClick);
                return SessionPickerOutput {
                    supported: true,
                    disabled: resolved.disabled,
                    chosen_model: resolved.chosen_model,
                    options_model: resolved.options_model,
                    note: resolved.note,
                    disabled_reason: resolved.disabled_reason,
                    closed: self.pending_close.take(),
                    escape_consumed,
                    ..Default::default()
                };
            }
        }

        let mut event = None;
        let mut focus_ids = Vec::new();
        let mut was_sizing_pass = false;
        let area_id = id_source.with(("pane-model-menu", input.agent_id));
        let request_initial_focus = open_state.needs_initial_focus;

        egui::Area::new(area_id)
            .order(Order::Tooltip)
            .fixed_pos(popup_rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                was_sizing_pass = ui.is_sizing_pass();
                let colors = resolve_colors(ui);
                egui::Frame::NONE
                    .fill(colors.surface_2)
                    .stroke(Stroke::new(1.0_f32, colors.border_strong))
                    .corner_radius(CornerRadius::same(9))
                    .inner_margin(Margin::same(POPOVER_OUTER_PADDING_I8))
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        let (ev, ids) = self.render_sop_contents(
                            ui,
                            SopRenderParams {
                                id_base: area_id.with("sop"),
                                width: inner_w,
                                max_height: max_sop_h,
                                request_initial_focus,
                                resolved: &resolved,
                            },
                        );
                        event = ev;
                        focus_ids = ids;
                    });
            });

        if let Some(open) = self.popover.as_mut()
            && request_initial_focus
            && !was_sizing_pass
        {
            open.needs_initial_focus = false;
        }
        self.last_popup_rect = Some(popup_rect);

        SessionPickerOutput {
            supported: true,
            disabled: resolved.disabled,
            chosen_model: resolved.chosen_model,
            options_model: resolved.options_model,
            note: resolved.note,
            disabled_reason: resolved.disabled_reason,
            event,
            closed: self.pending_close.take(),
            escape_consumed,
            popup_rect: Some(popup_rect),
            focus_ids,
        }
    }

    fn render_sop_contents(
        &mut self,
        ui: &mut Ui,
        params: SopRenderParams<'_>,
    ) -> (Option<SessionPickerEvent>, Vec<Id>) {
        let SopRenderParams {
            id_base,
            width,
            max_height,
            request_initial_focus,
            resolved,
        } = params;

        let interactive_count = resolved
            .items
            .iter()
            .filter(|it| it.is_interactive())
            .count();
        let default_active = resolved.initial_active_index();
        let mut active_idx = self
            .active_index
            .unwrap_or(default_active)
            .min(interactive_count.saturating_sub(1));

        let mut focus_ids: Vec<Id> = (0..interactive_count)
            .map(|idx| id_base.with(("sop-item", idx)))
            .collect();
        if request_initial_focus
            && !resolved.disabled
            && let Some(&target_id) = focus_ids.get(active_idx)
        {
            ui.ctx().memory_mut(|m| m.request_focus(target_id));
        }

        for (idx, &fid) in focus_ids.iter().enumerate() {
            if ui.memory(|m| m.has_focus(fid)) {
                active_idx = idx;
            }
        }

        let is_sizing = ui.is_sizing_pass();
        let parent_enabled = ui.is_enabled();
        let has_any_focus =
            focus_ids.iter().any(|&fid| ui.memory(|m| m.has_focus(fid))) || self.popover.is_some();
        let mut keyboard_activated: Option<usize> = None;

        if !is_sizing
            && !resolved.disabled
            && parent_enabled
            && interactive_count > 0
            && has_any_focus
            && ui.ctx().input(|i| i.focused)
        {
            let ctx = ui.ctx();
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                // Cancel egui's already scheduled navigation: this picker moves focus itself.
                ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                let next = (active_idx + 1).min(interactive_count - 1);
                if next != active_idx {
                    active_idx = next;
                    self.scroll_to_active = true;
                    ctx.memory_mut(|m| m.request_focus(focus_ids[active_idx]));
                }
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                // Cancel egui's already scheduled navigation: this picker moves focus itself.
                ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                let prev = active_idx.saturating_sub(1);
                if prev != active_idx {
                    active_idx = prev;
                    self.scroll_to_active = true;
                    ctx.memory_mut(|m| m.request_focus(focus_ids[active_idx]));
                }
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
                active_idx = 0;
                self.scroll_to_active = true;
                ctx.memory_mut(|m| m.request_focus(focus_ids[0]));
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::End)) {
                active_idx = interactive_count - 1;
                self.scroll_to_active = true;
                ctx.memory_mut(|m| m.request_focus(focus_ids[active_idx]));
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
                || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space))
            {
                keyboard_activated = Some(active_idx);
            }
        }

        let pointer_moved = ui.ctx().input(|i| i.pointer.delta() != Vec2::ZERO);
        let mut emitted: Option<SessionPickerEvent> = None;

        ui.set_min_width(width);
        ui.set_max_width(width);

        egui::ScrollArea::vertical()
            .id_salt(id_base.with("sop-scroll"))
            .max_height(max_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.set_min_width(width);
                ui.set_max_width(width);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(SOP_VERTICAL_PAD);

                let colors = resolve_colors(ui);
                let mut interactive_cursor = 0usize;

                for item in &resolved.items {
                    match item {
                        PickerLayoutItem::SectionLabel(text) => {
                            paint_sop_label(ui, width, text, colors.text_dim);
                        }
                        PickerLayoutItem::Note(text) => {
                            paint_sop_note(ui, width, text, colors.text_dim);
                        }
                        PickerLayoutItem::Separator => {
                            paint_sop_separator(ui, width, colors.border);
                        }
                        PickerLayoutItem::ActionRow { label, option_id } => {
                            let idx = interactive_cursor;
                            interactive_cursor += 1;
                            let row_id = focus_ids[idx];
                            let clicked = paint_sop_action_row(
                                ui,
                                ActionRowPaint {
                                    id: row_id,
                                    row_index: idx,
                                    width,
                                    label,
                                    disabled: resolved.disabled,
                                    is_active: !resolved.disabled && idx == active_idx,
                                    pointer_moved,
                                    scroll_to_me: self.scroll_to_active && idx == active_idx,
                                    colors,
                                },
                                &mut active_idx,
                            );
                            if !is_sizing && self.scroll_to_active && idx == active_idx {
                                self.scroll_to_active = false;
                            }
                            if !is_sizing
                                && parent_enabled
                                && !resolved.disabled
                                && (clicked || keyboard_activated == Some(idx))
                            {
                                emitted = Some(SessionPickerEvent::Action {
                                    option_id: option_id.clone(),
                                });
                            }
                        }
                        PickerLayoutItem::RadioRow {
                            label,
                            description,
                            checked,
                            option_id,
                            value,
                        } => {
                            let idx = interactive_cursor;
                            interactive_cursor += 1;
                            let row_id = focus_ids[idx];
                            let clicked = paint_sop_radio_row(
                                ui,
                                RadioRowPaint {
                                    id: row_id,
                                    row_index: idx,
                                    width,
                                    label,
                                    description,
                                    checked: *checked,
                                    disabled: resolved.disabled,
                                    is_active: !resolved.disabled && idx == active_idx,
                                    pointer_moved,
                                    scroll_to_me: self.scroll_to_active && idx == active_idx,
                                    colors,
                                },
                                &mut active_idx,
                            );
                            if !is_sizing && self.scroll_to_active && idx == active_idx {
                                self.scroll_to_active = false;
                            }
                            if !is_sizing
                                && parent_enabled
                                && !resolved.disabled
                                && (clicked || keyboard_activated == Some(idx))
                            {
                                emitted = Some(SessionPickerEvent::Set {
                                    option_id: option_id.clone(),
                                    value: value.clone(),
                                });
                            }
                        }
                        PickerLayoutItem::SwitchRow {
                            label,
                            checked,
                            option_id,
                        } => {
                            let idx = interactive_cursor;
                            interactive_cursor += 1;
                            let row_id = focus_ids[idx];
                            let clicked = paint_sop_switch_row(
                                ui,
                                SwitchRowPaint {
                                    id: row_id,
                                    row_index: idx,
                                    width,
                                    label,
                                    checked: *checked,
                                    disabled: resolved.disabled,
                                    is_active: !resolved.disabled && idx == active_idx,
                                    pointer_moved,
                                    scroll_to_me: self.scroll_to_active && idx == active_idx,
                                    colors,
                                },
                                &mut active_idx,
                            );
                            if !is_sizing && self.scroll_to_active && idx == active_idx {
                                self.scroll_to_active = false;
                            }
                            if !is_sizing
                                && parent_enabled
                                && !resolved.disabled
                                && (clicked || keyboard_activated == Some(idx))
                            {
                                emitted = Some(SessionPickerEvent::Set {
                                    option_id: option_id.clone(),
                                    value: Some(Value::Bool(!*checked)),
                                });
                            }
                        }
                    }
                }

                ui.add_space(SOP_VERTICAL_PAD);
            });

        if !is_sizing {
            self.active_index = (interactive_count > 0).then_some(active_idx);
        }
        if resolved.disabled || !parent_enabled {
            focus_ids.clear();
        }
        (emitted, focus_ids)
    }
}

// -----------------------------------------------------------------------------
// 4. Capability-Gated Pure Bridge to `pane_session_model`
// -----------------------------------------------------------------------------

/// Declarative result of [`plan_picker_event`], combining [`ModelPickPlan`] with
/// exact `TerminalPane.vue:871-930` popover close & terminal-refocus rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerPlanResult {
    pub plan: ModelPickPlan,
    /// `true` when the picker popover closed as a result of this event (`closeModelMenu(true)`).
    pub closed_picker: bool,
    /// `true` when the caller should return keyboard focus to the pane terminal.
    pub refocus_pane: bool,
}

/// Bridges a [`SessionPickerEvent`] to [`plan_model_pick`] / [`plan_model_action`] with
/// strict capability gating and `user_typing` protection (`zero I/O`).
///
/// Matches `TerminalPane.vue:871-930`:
/// - `Saved` / `Busy` / `None` outcomes keep the popover open so users can pick both a model
///   and an effort/switch in one session.
/// - Live outcomes (`Sent`, `Picker`, `Applied`, `Rejected`, `Unknown`, `AwaitClaudeConfirmation`,
///   `ActionSent`) close the popover and request terminal refocus (`refocus_pane: true`).
pub fn plan_picker_event(
    picker: &mut SessionOptionPickerUi,
    event: &SessionPickerEvent,
    node: &PaneModelNode,
    ctx: &PanePickContext<'_>,
    capabilities: SessionPickerCapabilities,
    claude_outcome: Option<ClaudeSwitchOutcome>,
) -> PickerPlanResult {
    match event {
        SessionPickerEvent::Set { option_id, value } => {
            let effective_ctx = PanePickContext {
                pane_running: ctx.pane_running && capabilities.live_command_switch,
                ..*ctx
            };
            let mut plan = plan_model_pick(
                node,
                &effective_ctx,
                option_id,
                value.as_ref(),
                claude_outcome,
            );
            // Protect any partially typed terminal line from live slash-command injection.
            if effective_ctx.pane_running && ctx.user_typing && !plan.typed_commands.is_empty() {
                let busy_ctx = PanePickContext {
                    pane_busy: true,
                    ..effective_ctx
                };
                let mut busy_plan =
                    plan_model_pick(node, &busy_ctx, option_id, value.as_ref(), None);
                busy_plan.outcome = Some(ModelPickOutcome::Busy);
                busy_plan.typed_commands.clear();
                busy_plan.toast = Some(PlannedToast {
                    text: typed_line_reason(ctx.french).to_owned(),
                    kind: ToastKind::Info,
                    timeout: Some(4_000),
                });
                busy_plan.hold_claude_default_model = false;
                busy_plan.pending_next_choice = None;
                busy_plan.node = node.clone();
                plan = busy_plan;
            }
            let close_menu = matches!(
                plan.outcome,
                Some(
                    ModelPickOutcome::Sent
                        | ModelPickOutcome::Picker
                        | ModelPickOutcome::Applied
                        | ModelPickOutcome::Rejected
                        | ModelPickOutcome::Unknown
                        | ModelPickOutcome::AwaitClaudeConfirmation
                )
            );
            if close_menu {
                picker.close();
            }
            PickerPlanResult {
                plan,
                closed_picker: close_menu,
                refocus_pane: close_menu,
            }
        }
        SessionPickerEvent::Action { option_id } => {
            if ctx.user_typing {
                return PickerPlanResult {
                    plan: blocked_action_plan(node, Some(typed_line_reason(ctx.french))),
                    closed_picker: false,
                    refocus_pane: false,
                };
            }
            let raw_plan = plan_model_action(node, ctx, option_id);
            let allowed = action_allowed_by_capabilities(node, ctx, option_id, capabilities);
            if !allowed {
                return PickerPlanResult {
                    plan: blocked_action_plan(node, None),
                    closed_picker: false,
                    refocus_pane: false,
                };
            }
            let close_menu = raw_plan.outcome == Some(ModelPickOutcome::ActionSent);
            if close_menu {
                picker.close();
            }
            PickerPlanResult {
                plan: raw_plan,
                closed_picker: close_menu,
                refocus_pane: close_menu,
            }
        }
    }
}

fn blocked_action_plan(node: &PaneModelNode, reason: Option<&str>) -> ModelPickPlan {
    ModelPickPlan {
        outcome: None,
        typed_commands: Vec::new(),
        toast: reason.map(|txt| PlannedToast {
            text: txt.to_owned(),
            kind: ToastKind::Info,
            timeout: Some(4_000),
        }),
        hold_claude_default_model: false,
        pending_next_choice: None,
        launch_stale_after: false,
        pane_menu_values_before: None,
        effective_id_before: None,
        node: node.clone(),
    }
}

fn action_allowed_by_capabilities(
    node: &PaneModelNode,
    ctx: &PanePickContext<'_>,
    option_id: &str,
    capabilities: SessionPickerCapabilities,
) -> bool {
    let Some(cat) = catalog::find(&node.agent_id) else {
        return false;
    };
    if option_id == "model" {
        return match cat.model_apply["midSession"]["kind"].as_str() {
            Some("agent-picker") => capabilities.live_agent_picker,
            Some("toggle-command") => capabilities.live_toggle_command,
            _ => false,
        };
    }
    let models = catalog::models_for(&node.agent_id, ctx.discovered_models);
    let pane_vals = launch_session_values(
        node.session_options.as_ref(),
        ctx.persisted_defaults,
        &node.agent_id,
    );
    let model_id = pane_vals
        .as_ref()
        .and_then(|v| v.get("model"))
        .and_then(Value::as_str)
        .or_else(|| effective_model_id(&models, ctx.agent_model.map(|m| m.model.as_str())))
        .unwrap_or("");
    let Some(opt) = catalog::model_options(cat, Some(models.as_slice()), model_id)
        .iter()
        .find(|o| o["id"].as_str() == Some(option_id))
    else {
        return false;
    };
    match opt["apply"]["midSession"]["kind"].as_str() {
        Some("toggle-command") => capabilities.live_toggle_command,
        Some("agent-picker") => capabilities.live_agent_picker,
        _ => false,
    }
}

// -----------------------------------------------------------------------------
// 5. Internal View Resolution, Clamping & Painting Helpers
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum PickerLayoutItem {
    SectionLabel(String),
    Note(String),
    Separator,
    ActionRow {
        label: String,
        option_id: String,
    },
    RadioRow {
        label: String,
        description: String,
        checked: bool,
        option_id: String,
        value: Option<Value>,
    },
    SwitchRow {
        label: String,
        checked: bool,
        option_id: String,
    },
}

impl PickerLayoutItem {
    const fn is_interactive(&self) -> bool {
        matches!(
            self,
            Self::ActionRow { .. } | Self::RadioRow { .. } | Self::SwitchRow { .. }
        )
    }
}

struct ResolvedPickerView {
    chosen_model: Option<String>,
    options_model: Option<String>,
    note: String,
    disabled_reason: String,
    disabled: bool,
    items: Vec<PickerLayoutItem>,
}

impl ResolvedPickerView {
    fn initial_active_index(&self) -> usize {
        let mut idx = 0usize;
        for item in &self.items {
            if let PickerLayoutItem::RadioRow {
                checked: true,
                option_id,
                ..
            } = item
                && option_id == "model"
            {
                return idx;
            }
            if item.is_interactive() {
                idx += 1;
            }
        }
        0
    }
}

struct SopRenderParams<'a> {
    id_base: Id,
    width: f32,
    max_height: f32,
    request_initial_focus: bool,
    resolved: &'a ResolvedPickerView,
}

fn strip_trailing_1m_bracket_ci(value: &str) -> &str {
    match value.as_bytes() {
        [prefix @ .., b'[', b'1', b'm' | b'M', b']'] => &value[..prefix.len()],
        _ => value,
    }
}

fn picker_shown_values(models: &[Value], values: Option<&Value>) -> Option<Map<String, Value>> {
    let obj = values?.as_object()?;
    if obj
        .get("model")
        .and_then(Value::as_str)
        .is_some_and(|m| !m.is_empty())
    {
        return values_on_listed_row(models, values);
    }
    Some(obj.clone())
}

fn resolve_chosen_row_id(models: &[Value], raw_model: Option<&str>) -> Option<String> {
    let model = raw_model.filter(|s| !s.is_empty())?;
    if models.iter().any(|m| m["id"].as_str() == Some(model)) {
        return Some(model.to_owned());
    }
    let name = model_label(model);
    let plain = name.strip_suffix(" (1M)").unwrap_or(&name);
    let stripped = strip_trailing_1m_bracket_ci(model);
    let matched = models
        .iter()
        .find(|m| {
            m["label"]
                .as_str()
                .is_some_and(|lbl| !lbl.is_empty() && (lbl == name || lbl == plain))
        })
        .or_else(|| models.iter().find(|m| m["id"].as_str() == Some(stripped)))
        .and_then(|m| m["id"].as_str());
    Some(matched.unwrap_or(model).to_owned())
}

fn agent_has_wired_live_terminal_commands(
    cat: &catalog::Catalog,
    options: &[Value],
    capabilities: SessionPickerCapabilities,
) -> bool {
    let model_mid = cat.model_apply["midSession"]["kind"].as_str();
    if capabilities.live_command_switch && matches!(model_mid, Some("command" | "picker-filter")) {
        return true;
    }
    if capabilities.live_agent_picker && model_mid == Some("agent-picker") {
        return true;
    }
    options
        .iter()
        .any(|opt| match opt["apply"]["midSession"]["kind"].as_str() {
            Some("command" | "picker-filter") => capabilities.live_command_switch,
            Some("toggle-command") => capabilities.live_toggle_command,
            Some("agent-picker") => capabilities.live_agent_picker,
            _ => false,
        })
}

fn resolve_picker_view(input: &SessionPickerInput<'_>) -> Option<ResolvedPickerView> {
    if !has_model_choice(input.is_agent, input.detected, input.agent_id) {
        return None;
    }
    let cat = catalog::find(input.agent_id)?;
    let models = catalog::models_for(input.agent_id, input.discovered_models);
    let menu_vals = pane_menu_values(
        input.agent_id,
        input.pane_running,
        &models,
        input.agent_model,
        input.session_options,
    );
    let shown_values = picker_shown_values(&models, menu_vals.as_ref());
    let raw_model = shown_values
        .as_ref()
        .and_then(|m| m.get("model"))
        .and_then(Value::as_str);
    let chosen_model = resolve_chosen_row_id(&models, raw_model);

    let fallback_model = effective_model_id(&models, input.agent_model.map(|m| m.model.as_str()));
    let options_model = chosen_model
        .clone()
        .or_else(|| fallback_model.map(str::to_owned));
    let raw_options: &[Value] = match options_model.as_deref() {
        Some(m_id) => catalog::model_options(cat, Some(models.as_slice()), m_id),
        None => &[],
    };

    let note = capability_gated_menu_note(
        input.agent_id,
        input.pane_running,
        input.capabilities,
        input.french,
    )
    .to_owned();

    let disabled_reason =
        if let Some(override_reason) = input.disabled_reason_override.filter(|s| !s.is_empty()) {
            override_reason.to_owned()
        } else {
            let busy = model_busy_reason(input.pane_running, input.pane_busy, input.french);
            if !busy.is_empty() {
                busy.to_owned()
            } else if input.pane_running
                && input.user_typing
                && agent_has_wired_live_terminal_commands(cat, raw_options, input.capabilities)
            {
                typed_line_reason(input.french).to_owned()
            } else {
                String::new()
            }
        };
    let disabled = input.pending || !disabled_reason.is_empty();

    let mut items = Vec::with_capacity(6 + models.len() + raw_options.len() * 3);
    items.push(PickerLayoutItem::SectionLabel(
        model_field_label(input.french).to_owned(),
    ));
    if !note.is_empty() {
        items.push(PickerLayoutItem::Note(note.clone()));
    }
    if !disabled_reason.is_empty() {
        items.push(PickerLayoutItem::Note(disabled_reason.clone()));
    }

    let show_model_agent_picker = input.pane_running
        && input.capabilities.live_agent_picker
        && cat.model_apply["midSession"]["kind"].as_str() == Some("agent-picker");
    if show_model_agent_picker {
        items.push(PickerLayoutItem::ActionRow {
            label: choose_in_agent_picker_label(input.french).to_owned(),
            option_id: "model".to_owned(),
        });
    }

    if input.show_default_row {
        let def_label = model_default_label(
            input.agent_id,
            &models,
            input.persisted_defaults,
            input.french,
        );
        if !def_label.is_empty() {
            items.push(PickerLayoutItem::RadioRow {
                label: def_label,
                description: input.default_hint.unwrap_or_default().to_owned(),
                checked: chosen_model.is_none(),
                option_id: "model".to_owned(),
                value: None,
            });
        }
    }

    for model in &models {
        let Some(m_id) = model["id"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let label = model["label"].as_str().unwrap_or(m_id).to_owned();
        let desc = model_description(model, input.french).to_owned();
        items.push(PickerLayoutItem::RadioRow {
            label,
            description: desc,
            checked: chosen_model.as_deref() == Some(m_id),
            option_id: "model".to_owned(),
            value: Some(Value::String(m_id.to_owned())),
        });
    }

    if let Some(chosen) = chosen_model.as_deref()
        && !models.iter().any(|m| m["id"].as_str() == Some(chosen))
    {
        let fallback_lbl = model_label(chosen);
        let label = if fallback_lbl.is_empty() {
            chosen.to_owned()
        } else {
            fallback_lbl
        };
        items.push(PickerLayoutItem::RadioRow {
            label,
            description: String::new(),
            checked: true,
            option_id: "model".to_owned(),
            value: Some(Value::String(chosen.to_owned())),
        });
    }

    let active_model_row = options_model
        .as_deref()
        .and_then(|m_id| models.iter().find(|m| m["id"].as_str() == Some(m_id)));
    let has_grouped_variants = active_model_row.is_some_and(|m| m["variants"].is_array());

    for opt in raw_options {
        let mode = option_row_mode(input.agent_id, opt, input.pane_running, input.capabilities);
        if mode == OptionRowMode::Hidden {
            continue;
        }
        let Some(opt_id) = opt["id"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        items.push(PickerLayoutItem::Separator);

        match mode {
            OptionRowMode::Toggle => {
                items.push(PickerLayoutItem::ActionRow {
                    label: toggle_option_label(opt, input.french),
                    option_id: opt_id.to_owned(),
                });
            }
            OptionRowMode::AgentPicker => {
                items.push(PickerLayoutItem::SectionLabel(
                    session_option_label(opt, input.french).to_owned(),
                ));
                items.push(PickerLayoutItem::ActionRow {
                    label: choose_in_agent_picker_label(input.french).to_owned(),
                    option_id: opt_id.to_owned(),
                });
            }
            OptionRowMode::Switch => {
                let explicit = shown_values
                    .as_ref()
                    .and_then(|m| m.get(opt_id))
                    .and_then(Value::as_bool);
                let checked = explicit
                    .unwrap_or_else(|| has_grouped_variants && opt["kind"]["defaultValue"] == true);
                items.push(PickerLayoutItem::SwitchRow {
                    label: session_option_label(opt, input.french).to_owned(),
                    checked,
                    option_id: opt_id.to_owned(),
                });
            }
            OptionRowMode::Select => {
                items.push(PickerLayoutItem::SectionLabel(
                    session_option_label(opt, input.french).to_owned(),
                ));
                let cur_val = shown_values.as_ref().and_then(|m| m.get(opt_id));
                let is_default = cur_val.is_none_or(Value::is_null);
                items.push(PickerLayoutItem::RadioRow {
                    label: option_default_label(input.french).to_owned(),
                    description: String::new(),
                    checked: is_default,
                    option_id: opt_id.to_owned(),
                    value: None,
                });
                if let Some(choices) = opt["kind"]["choices"].as_array() {
                    for choice in choices {
                        if let Some(c_val) = choice["value"].as_str().filter(|s| !s.is_empty()) {
                            let is_checked = cur_val.and_then(Value::as_str) == Some(c_val);
                            items.push(PickerLayoutItem::RadioRow {
                                label: session_choice_label(choice, input.french).to_owned(),
                                description: String::new(),
                                checked: is_checked,
                                option_id: opt_id.to_owned(),
                                value: Some(Value::String(c_val.to_owned())),
                            });
                        }
                    }
                }
            }
            OptionRowMode::Hidden => {}
        }
    }

    Some(ResolvedPickerView {
        chosen_model,
        options_model,
        note,
        disabled_reason,
        disabled,
        items,
    })
}

fn measure_sop_natural_width(ctx: &egui::Context, resolved: &ResolvedPickerView) -> f32 {
    let mut max_w = PICKER_MIN_WIDTH;
    ctx.fonts_mut(|fonts| {
        for item in &resolved.items {
            let item_w = match item {
                PickerLayoutItem::SectionLabel(txt) => {
                    let w = fonts
                        .layout_no_wrap(txt.clone(), FontId::proportional(11.0), Color32::WHITE)
                        .size()
                        .x;
                    w + SOP_ROW_PAD_X * 2.0
                }
                PickerLayoutItem::Note(_) => PICKER_MAX_WIDTH,
                PickerLayoutItem::Separator => PICKER_MIN_WIDTH,
                PickerLayoutItem::ActionRow { label, .. } => {
                    let w = fonts
                        .layout_no_wrap(label.clone(), FontId::proportional(12.0), Color32::WHITE)
                        .size()
                        .x;
                    w + SOP_ROW_PAD_X * 2.0 + 4.0
                }
                PickerLayoutItem::RadioRow {
                    label, description, ..
                } => {
                    let name_w = fonts
                        .layout_no_wrap(label.clone(), FontId::proportional(12.0), Color32::WHITE)
                        .size()
                        .x
                        + SOP_ROW_PAD_X * 2.0
                        + SOP_CHECK_WIDTH
                        + SOP_CHECK_GAP
                        + 4.0;
                    if description.is_empty() {
                        name_w
                    } else {
                        name_w.max(268.0)
                    }
                }
                PickerLayoutItem::SwitchRow { label, .. } => {
                    let w = fonts
                        .layout_no_wrap(label.clone(), FontId::proportional(12.0), Color32::WHITE)
                        .size()
                        .x;
                    w + SOP_ROW_PAD_X * 2.0 + 8.0 + SWITCH_WIDTH
                }
            };
            max_w = max_w.max(item_w.ceil());
        }
    });
    max_w.clamp(PICKER_MIN_WIDTH, PICKER_MAX_WIDTH)
}

fn estimate_sop_content_height(
    ctx: &egui::Context,
    inner_w: f32,
    resolved: &ResolvedPickerView,
) -> f32 {
    let mut total = SOP_VERTICAL_PAD * 2.0;
    for item in &resolved.items {
        total += match item {
            PickerLayoutItem::SectionLabel(_) => SOP_LABEL_HEIGHT,
            PickerLayoutItem::Note(text) => {
                let wrap_w = (inner_w - SOP_ROW_PAD_X * 2.0).max(24.0);
                wrapped_text_height(ctx, text, FontId::proportional(11.0), wrap_w) + 4.0
            }
            PickerLayoutItem::Separator => SOP_SEP_HEIGHT,
            PickerLayoutItem::ActionRow { .. } => SOP_SIMPLE_ROW_HEIGHT,
            PickerLayoutItem::RadioRow { description, .. } => {
                if description.is_empty() {
                    SOP_SIMPLE_ROW_HEIGHT
                } else {
                    let desc_w =
                        (inner_w - SOP_ROW_PAD_X * 2.0 - SOP_CHECK_WIDTH - SOP_CHECK_GAP).max(24.0);
                    let desc_h =
                        wrapped_text_height(ctx, description, FontId::proportional(11.0), desc_w);
                    SOP_ROW_PAD_Y * 2.0 + 16.0 + 1.0 + desc_h
                }
            }
            PickerLayoutItem::SwitchRow { .. } => SOP_SWITCH_ROW_HEIGHT,
        };
    }
    total.ceil()
}

fn compute_clamped_popover_rect(
    ctx: &egui::Context,
    anchor: Pos2,
    resolved: &ResolvedPickerView,
) -> (Rect, f32, f32) {
    let viewport = ctx.content_rect();
    let max_outer_w = (viewport.width() - PICKER_VIEWPORT_MARGIN * 2.0).max(76.0);
    let max_inner_w = (max_outer_w - POPOVER_OUTER_PADDING * 2.0).max(60.0);
    let natural_inner_w = measure_sop_natural_width(ctx, resolved);
    let inner_w = natural_inner_w.clamp(
        PICKER_MIN_WIDTH.min(max_inner_w),
        PICKER_MAX_WIDTH.min(max_inner_w),
    );

    let max_outer_h = (viewport.height() - PICKER_VIEWPORT_MARGIN * 2.0).max(56.0);
    let max_sop_h = (viewport.height() * 0.70)
        .min(PICKER_MAX_HEIGHT)
        .min((max_outer_h - POPOVER_OUTER_PADDING * 2.0).max(40.0));
    let content_h = estimate_sop_content_height(ctx, inner_w, resolved);
    let sop_h = content_h.min(max_sop_h);

    let outer_w = inner_w + POPOVER_OUTER_PADDING * 2.0;
    let outer_h = sop_h + POPOVER_OUTER_PADDING * 2.0;

    let mut x = anchor.x;
    let mut y = anchor.y;
    if x + outer_w > viewport.right() {
        x = viewport.right() - outer_w - PICKER_VIEWPORT_MARGIN;
    }
    if y + outer_h > viewport.bottom() {
        y = viewport.bottom() - outer_h - PICKER_VIEWPORT_MARGIN;
    }
    x = x.max(viewport.left() + PICKER_VIEWPORT_MARGIN);
    y = y.max(viewport.top() + PICKER_VIEWPORT_MARGIN);

    (
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(outer_w, outer_h)),
        inner_w,
        max_sop_h,
    )
}

#[derive(Clone, Copy)]
struct ResolvedColors {
    surface_2: Color32,
    surface_3: Color32,
    border: Color32,
    border_strong: Color32,
    accent: Color32,
    text: Color32,
    text_strong: Color32,
    text_dim: Color32,
}

fn resolve_colors(ui: &Ui) -> ResolvedColors {
    if ui.visuals().dark_mode {
        let p = &theme::active(ui.ctx()).spec().ui;
        ResolvedColors {
            surface_2: p.surface_2,
            surface_3: p.surface_3,
            border: p.border,
            border_strong: p.border_strong,
            accent: p.accent,
            text: p.text,
            text_strong: p.text_strong,
            text_dim: p.text_dim,
        }
    } else {
        let v = ui.visuals();
        ResolvedColors {
            surface_2: v.window_fill,
            surface_3: v.selection.bg_fill,
            border: v.widgets.noninteractive.bg_stroke.color,
            border_strong: v.widgets.inactive.bg_stroke.color,
            accent: v.selection.stroke.color,
            text: v.text_color(),
            text_strong: v.strong_text_color(),
            text_dim: v.weak_text_color(),
        }
    }
}

fn dim_if_disabled(color: Color32, disabled: bool) -> Color32 {
    if disabled {
        color.gamma_multiply(DISABLED_ALPHA_FACTOR)
    } else {
        color
    }
}

fn paint_sop_label(ui: &mut Ui, width: f32, text: &str, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, SOP_LABEL_HEIGHT), Sense::hover());
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + SOP_ROW_PAD_X, rect.top() + 4.0),
        Pos2::new(
            (rect.right() - SOP_ROW_PAD_X).max(rect.left() + SOP_ROW_PAD_X),
            rect.bottom(),
        ),
    );
    paint_single_line(ui, text_rect, text, FontId::proportional(11.0), color);
}

fn paint_sop_note(ui: &mut Ui, width: f32, text: &str, color: Color32) {
    let wrap_w = (width - SOP_ROW_PAD_X * 2.0).max(24.0);
    let galley = layout_wrapped(ui.ctx(), text, FontId::proportional(11.0), color, wrap_w);
    let height = galley.size().y.ceil().max(14.0) + 4.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(
            Pos2::new(rect.left() + SOP_ROW_PAD_X, rect.top()),
            galley,
            color,
        );
}

fn paint_sop_separator(ui: &mut Ui, width: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, SOP_SEP_HEIGHT), Sense::hover());
    let y = rect.center().y;
    ui.painter().line_segment(
        [
            Pos2::new(rect.left() + 6.0, y),
            Pos2::new((rect.right() - 6.0).max(rect.left() + 6.0), y),
        ],
        Stroke::new(1.0_f32, color),
    );
}

struct ActionRowPaint<'a> {
    id: Id,
    row_index: usize,
    width: f32,
    label: &'a str,
    disabled: bool,
    is_active: bool,
    pointer_moved: bool,
    scroll_to_me: bool,
    colors: ResolvedColors,
}

fn paint_sop_action_row(ui: &mut Ui, spec: ActionRowPaint<'_>, active_idx: &mut usize) -> bool {
    let enabled = ui.is_enabled() && !spec.disabled;
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(spec.width, SOP_SIMPLE_ROW_HEIGHT), Sense::hover());
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, spec.id, sense);
    let a11y = spec.label.to_owned();
    response.widget_info(move || WidgetInfo::labeled(WidgetType::Button, enabled, &a11y));

    if response.has_focus() {
        lock_row_keys(ui, spec.id);
    }
    if enabled && spec.pointer_moved && response.hovered() {
        *active_idx = spec.row_index;
    }
    if spec.scroll_to_me && !ui.is_sizing_pass() && !ui.clip_rect().contains_rect(rect) {
        response.scroll_to_me(Some(Align::Center));
    }

    let hovered = enabled && (response.hovered() || spec.is_active || response.has_focus());
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), spec.colors.surface_3);
    }
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + SOP_ROW_PAD_X, rect.top()),
        Pos2::new(
            (rect.right() - SOP_ROW_PAD_X).max(rect.left() + SOP_ROW_PAD_X),
            rect.bottom(),
        ),
    );
    paint_single_line(
        ui,
        text_rect,
        spec.label,
        FontId::proportional(12.0),
        dim_if_disabled(spec.colors.text, !enabled),
    );

    let response = if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    let clicked = enabled && response.clicked();
    if clicked {
        response.request_focus();
    }
    clicked
}

struct RadioRowPaint<'a> {
    id: Id,
    row_index: usize,
    width: f32,
    label: &'a str,
    description: &'a str,
    checked: bool,
    disabled: bool,
    is_active: bool,
    pointer_moved: bool,
    scroll_to_me: bool,
    colors: ResolvedColors,
}

fn paint_sop_radio_row(ui: &mut Ui, spec: RadioRowPaint<'_>, active_idx: &mut usize) -> bool {
    let enabled = ui.is_enabled() && !spec.disabled;
    let body_left_offset = SOP_ROW_PAD_X + SOP_CHECK_WIDTH + SOP_CHECK_GAP;
    let body_w = (spec.width - body_left_offset - SOP_ROW_PAD_X).max(24.0);
    let desc_color = dim_if_disabled(spec.colors.text_dim, !enabled);
    let desc_galley = (!spec.description.is_empty()).then(|| {
        layout_wrapped(
            ui.ctx(),
            spec.description,
            FontId::proportional(11.0),
            desc_color,
            body_w,
        )
    });
    let row_h = match &desc_galley {
        Some(galley) => (SOP_ROW_PAD_Y * 2.0 + 16.0 + 1.0 + galley.size().y).ceil(),
        None => SOP_SIMPLE_ROW_HEIGHT,
    };

    let (rect, _) = ui.allocate_exact_size(Vec2::new(spec.width, row_h), Sense::hover());
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, spec.id, sense);
    let a11y = spec.label.to_owned();
    let checked = spec.checked;
    response.widget_info(move || {
        WidgetInfo::selected(WidgetType::RadioButton, enabled, checked, &a11y)
    });

    if response.has_focus() {
        lock_row_keys(ui, spec.id);
    }
    if enabled && spec.pointer_moved && response.hovered() {
        *active_idx = spec.row_index;
    }
    if spec.scroll_to_me && !ui.is_sizing_pass() && !ui.clip_rect().contains_rect(rect) {
        response.scroll_to_me(Some(Align::Center));
    }

    let hovered = enabled && (response.hovered() || spec.is_active || response.has_focus());
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), spec.colors.surface_3);
    }

    let name_color = dim_if_disabled(
        if spec.checked {
            spec.colors.text_strong
        } else {
            spec.colors.text
        },
        !enabled,
    );
    let first_line_rect = Rect::from_min_size(
        Pos2::new(rect.left() + SOP_ROW_PAD_X, rect.top() + SOP_ROW_PAD_Y),
        Vec2::new(SOP_CHECK_WIDTH, 16.0),
    );
    if spec.checked {
        paint_single_line(
            ui,
            first_line_rect,
            "●",
            FontId::proportional(8.5),
            name_color,
        );
    }

    let name_rect = Rect::from_min_size(
        Pos2::new(rect.left() + body_left_offset, rect.top() + SOP_ROW_PAD_Y),
        Vec2::new(body_w, 16.0),
    );
    paint_single_line(
        ui,
        name_rect,
        spec.label,
        FontId::proportional(12.0),
        name_color,
    );

    if let Some(galley) = desc_galley {
        let desc_pos = Pos2::new(
            rect.left() + body_left_offset,
            rect.top() + SOP_ROW_PAD_Y + 17.0,
        );
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .galley(desc_pos, galley, desc_color);
    }

    let response = if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    let clicked = enabled && response.clicked();
    if clicked {
        response.request_focus();
    }
    clicked
}

struct SwitchRowPaint<'a> {
    id: Id,
    row_index: usize,
    width: f32,
    label: &'a str,
    checked: bool,
    disabled: bool,
    is_active: bool,
    pointer_moved: bool,
    scroll_to_me: bool,
    colors: ResolvedColors,
}

fn paint_sop_switch_row(ui: &mut Ui, spec: SwitchRowPaint<'_>, active_idx: &mut usize) -> bool {
    let enabled = ui.is_enabled() && !spec.disabled;
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(spec.width, SOP_SWITCH_ROW_HEIGHT), Sense::hover());
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, spec.id, sense);
    let a11y = spec.label.to_owned();
    let checked = spec.checked;
    response
        .widget_info(move || WidgetInfo::selected(WidgetType::Checkbox, enabled, checked, &a11y));

    if response.has_focus() {
        lock_row_keys(ui, spec.id);
    }
    if enabled && spec.pointer_moved && response.hovered() {
        *active_idx = spec.row_index;
    }
    if spec.scroll_to_me && !ui.is_sizing_pass() && !ui.clip_rect().contains_rect(rect) {
        response.scroll_to_me(Some(Align::Center));
    }

    let hovered = enabled && (response.hovered() || spec.is_active || response.has_focus());
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(4), spec.colors.surface_3);
    }

    let switch_w = SWITCH_WIDTH.min((rect.width() - SOP_ROW_PAD_X * 2.0).max(20.0));
    let switch_rect = Rect::from_min_size(
        Pos2::new(
            (rect.right() - SOP_ROW_PAD_X - switch_w).max(rect.left() + SOP_ROW_PAD_X),
            rect.center().y - SWITCH_HEIGHT * 0.5,
        ),
        Vec2::new(switch_w, SWITCH_HEIGHT),
    );
    let label_rect = Rect::from_min_max(
        Pos2::new(rect.left() + SOP_ROW_PAD_X, rect.top()),
        Pos2::new(
            (switch_rect.left() - 8.0).max(rect.left() + SOP_ROW_PAD_X),
            rect.bottom(),
        ),
    );
    paint_single_line(
        ui,
        label_rect,
        spec.label,
        FontId::proportional(12.0),
        dim_if_disabled(spec.colors.text, !enabled),
    );

    let pill_fill = dim_if_disabled(
        if spec.checked {
            spec.colors.accent
        } else {
            spec.colors.surface_3
        },
        !enabled,
    );
    ui.painter()
        .rect_filled(switch_rect, CornerRadius::same(10), pill_fill);
    if !spec.checked {
        ui.painter().rect_stroke(
            switch_rect,
            CornerRadius::same(10),
            Stroke::new(
                1.0_f32,
                dim_if_disabled(spec.colors.border_strong, !enabled),
            ),
            StrokeKind::Inside,
        );
    }
    let knob_x = if spec.checked {
        (switch_rect.right() - 10.0).max(switch_rect.center().x)
    } else {
        (switch_rect.left() + 10.0).min(switch_rect.center().x)
    };
    let knob_color = dim_if_disabled(
        if spec.checked {
            Color32::WHITE
        } else {
            Color32::from_rgb(0xae, 0xb4, 0xbf)
        },
        !enabled,
    );
    ui.painter()
        .circle_filled(Pos2::new(knob_x, switch_rect.center().y), 7.0, knob_color);

    let response = if enabled {
        response.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        response
    };
    let clicked = enabled && response.clicked();
    if clicked {
        response.request_focus();
    }
    clicked
}

fn lock_row_keys(ui: &mut Ui, id: Id) {
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            EventFilter {
                escape: true,
                vertical_arrows: true,
                ..Default::default()
            },
        );
    });
}

fn measure_text_width(ui: &mut Ui, text: &str, font: FontId) -> f32 {
    ui.fonts_mut(|f| {
        f.layout_no_wrap(text.to_owned(), font, Color32::WHITE)
            .size()
            .x
    })
}

fn layout_wrapped(
    ctx: &egui::Context,
    text: &str,
    font: FontId,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, max_width.max(16.0));
    job.wrap.break_anywhere = false;
    ctx.fonts_mut(|f| f.layout_job(job))
}

fn wrapped_text_height(ctx: &egui::Context, text: &str, font: FontId, max_width: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    layout_wrapped(ctx, text, font, Color32::WHITE, max_width)
        .size()
        .y
        .ceil()
}

fn paint_single_line(ui: &Ui, rect: Rect, text: &str, font: FontId, color: Color32) {
    if rect.width() <= 1.0 || rect.height() <= 1.0 || text.is_empty() {
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
    let galley = ui.ctx().fonts_mut(|f| f.layout_job(job));
    let pos = Pos2::new(rect.left(), rect.center().y - galley.size().y * 0.5);
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(pos, galley, color);
}

#[cfg(test)]
mod navigation_regression_tests {
    use super::*;

    #[test]
    fn each_arrow_moves_one_row_and_stops_at_the_boundary() {
        let ctx = egui::Context::default();
        let mut picker = SessionOptionPickerUi::new();
        picker.open_at_point(&ctx, Pos2::new(20.0, 20.0), None);
        let frame = |picker: &mut SessionOptionPickerUi, key: Option<Key>| {
            let mut output = SessionPickerOutput::default();
            let defaults = BTreeMap::new();
            let events = key
                .into_iter()
                .map(|key| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                })
                .collect();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    events,
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    let mut input = SessionPickerInput::new("claude", &defaults);
                    input.capabilities = SessionPickerCapabilities::restart_only();
                    output = picker.show_popover(ctx, Id::new("arrow-regression"), &input);
                },
            );
            output
                .focus_ids
                .iter()
                .position(|id| ctx.memory(|memory| memory.has_focus(*id)))
        };
        frame(&mut picker, None);
        frame(&mut picker, None);
        for (key, expected) in [
            (Key::Home, 0),
            (Key::ArrowDown, 1),
            (Key::ArrowDown, 2),
            (Key::ArrowUp, 1),
            (Key::ArrowUp, 0),
            (Key::ArrowUp, 0),
        ] {
            assert_eq!(frame(&mut picker, Some(key)), Some(expected), "{key:?}");
        }
        let last = frame(&mut picker, Some(Key::End)).unwrap();
        assert!(last > 2);
        assert_eq!(frame(&mut picker, Some(Key::ArrowDown)), Some(last));
    }
}
