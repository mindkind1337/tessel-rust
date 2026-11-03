// Adapted from Tessel AgentModelDefaults.vue, ThemedSelect.vue, sessionOptionLabels.js,
// cursorModels.js, and Orca (github.com/stablyai/orca, MIT, Copyright (c) 2026 Lovecast Inc.;
// see THIRD_PARTY_NOTICES.md).
//! Native `Settings > Agents` default model & session-option selector (`AgentModelDefaults.vue`),
//! `ThemedSelect`-faithful listbox popup, EN/FR option/choice/status labels (`sessionOptionLabels.js`),
//! and Cursor variant row projection (`valuesOnListedRow` / `decomposeCursorModel`).

use crate::{agent_model_catalog as catalog, agent_model_defaults::Defaults};
use eframe::egui::{
    self, Align, Color32, CornerRadius, EventFilter, FontId, Id, Key, Margin, Modifiers, Order,
    PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, WidgetType,
};
use serde_json::{Map, Value};

const FIELD_WIDTH: f32 = 220.0;
const CONTROL_HEIGHT: f32 = 28.0;
const HINT_HEIGHT: f32 = 15.0;
const LABEL_GAP: f32 = 2.0;
const FIELD_HEIGHT: f32 = HINT_HEIGHT + LABEL_GAP + CONTROL_HEIGHT;
const COL_GAP: f32 = 12.0;
const ROW_GAP: f32 = 6.0;
const SWITCH_WIDTH: f32 = 34.0;
const SWITCH_HEIGHT: f32 = 20.0;
const POPUP_MIN_WIDTH: f32 = 240.0;
const POPUP_MAX_HEIGHT: f32 = 320.0;
const POPUP_OPTION_HEIGHT: f32 = 28.0;
const POPUP_PADDING: f32 = 4.0;
const POPUP_PADDING_I8: i8 = 4;
const TYPEAHEAD_RESET_SECS: f64 = 0.7;

/// Runtime capabilities that gate status notes and actions until each host feature is wired.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Whether the `"Refresh models"` / `"Actualiser les modèles"` CLI probe action is wired.
    pub can_refresh_models: bool,
    /// Whether running panes expose `pane menu > Model` live switching (e.g. Claude `/fast`).
    pub running_pane_model_menu: bool,
    /// Whether running panes show `"Restart to apply"` when saved defaults change.
    pub running_pane_restart_badge: bool,
}

/// Summary metadata for a cached/probed CLI model list (`modelsListed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListedMeta<'a> {
    pub count: usize,
    pub date_label: &'a str,
}

/// Reason a CLI model probe produced no usable list (`sessionOptionLabels.js:probeErrorText`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProbeError<'a> {
    NotFound,
    Timeout,
    TooMuch,
    Empty,
    Unsupported,
    Failed { detail: Option<&'a str> },
}

/// Current CLI model-probe status for an agent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ProbeState<'a> {
    #[default]
    Idle,
    Busy,
    Error(ProbeError<'a>),
}

/// Frame input for [`AgentModelDefaultsUi::show`].
pub struct Input<'a> {
    pub agent_id: &'a str,
    pub agent_name: &'a str,
    pub defaults: &'a Defaults,
    /// Optional CLI-discovered catalog rows (merged via [`catalog::models_for`]).
    pub discovered_models: Option<&'a [Value]>,
    /// Metadata for the active discovered model list (`{{count}} models listed by {{name}} on {{date}}.`).
    pub listed_meta: Option<ListedMeta<'a>>,
    pub probe: ProbeState<'a>,
    pub capabilities: Capabilities,
    pub french: bool,
    /// Renders `.sl-section-head` (`"Default model"` / `"Modèle par défaut"`) above `.agent-models`.
    pub show_section_header: bool,
}

/// Frame output from [`AgentModelDefaultsUi::show`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// `true` when `agent_id` has a session-option catalog (`agent_model_catalog::find(agent_id).is_some()`).
    pub supported: bool,
    /// Emitted only when the user explicitly changed the model or an option (`next != *input.defaults`).
    pub changed: Option<Defaults>,
    /// `true` when the user clicked `"Refresh models"` / `"Actualiser les modèles"`.
    pub refresh_models: bool,
    /// Focusable widget IDs inside `.agent-models` for `AgentsPanel` detail-focus tracking.
    pub focus_ids: Vec<Id>,
    /// `true` when an open select popup consumed `Escape` during this frame.
    pub escape_consumed: bool,
}

#[derive(Clone, Debug)]
struct OpenSelect {
    trigger_id: Id,
    active: usize,
    scroll_to_active: bool,
    trigger_rect: Rect,
    popup_rect: Rect,
    search: String,
    last_search_time: f64,
}

/// Stateful `Settings > Agents` default model & effort picker (`AgentModelDefaults.vue` + `ThemedSelect.vue`).
#[derive(Clone, Debug, Default)]
pub struct AgentModelDefaultsUi {
    open_select: Option<OpenSelect>,
    open_agent: Option<String>,
    escape_held: bool,
    escape_consumed_pass: Option<u64>,
}

impl AgentModelDefaultsUi {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets transient popup and key-repeat state.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Closes any currently open select popup without clearing `escape_held`.
    pub fn close_popup(&mut self) {
        self.open_select = None;
    }

    /// Returns `true` when a `ThemedSelect` listbox popup is currently open.
    #[must_use]
    pub fn is_popup_open(&self) -> bool {
        self.open_select.is_some()
    }

    /// Intercepts `Escape` before parent detail-folding and before `egui::Modal` (`settings_surface::show`).
    ///
    /// - 1st `Escape` while a select listbox is open: closes only the listbox, restores focus to
    ///   its trigger button, arms key-repeat absorption, and returns `true`.
    /// - While `Escape` stays held after closing the listbox: absorbs repeat events and returns `true`.
    /// - When no listbox is open: returns `false` without consuming `Escape` so `AgentsPanel` folds
    ///   the open agent details (2nd `Escape`) and `Settings` closes on the 3rd `Escape`.
    pub fn handle_escape(&mut self, ctx: &egui::Context) -> bool {
        let down = ctx.input(|i| i.key_down(Key::Escape));
        if !down {
            self.escape_held = false;
        }
        if !ctx.input(|i| i.focused) {
            return false;
        }
        if self.open_select.is_none() && !self.escape_held {
            return false;
        }
        let consumed = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if !(consumed || down) {
            return false;
        }
        if let Some(open) = self.open_select.take() {
            ctx.memory_mut(|m| m.request_focus(open.trigger_id));
            self.escape_held = down;
            self.escape_consumed_pass = Some(ctx.cumulative_pass_nr());
            return true;
        }
        if consumed && self.escape_held {
            self.escape_consumed_pass = Some(ctx.cumulative_pass_nr());
            return true;
        }
        false
    }

    /// Renders `.agent-models` (and optional `.sl-section-head`) for `input.agent_id`.
    pub fn show(&mut self, ui: &mut Ui, input: Input<'_>) -> Output {
        let Some(cat) = catalog::find(input.agent_id) else {
            if self.open_agent.as_deref() == Some(input.agent_id) {
                self.open_select = None;
            }
            return Output::default();
        };

        if self.open_agent.as_deref() != Some(input.agent_id) {
            self.open_select = None;
            self.open_agent = Some(input.agent_id.to_owned());
        }

        if !ui.is_enabled() {
            self.open_select = None;
        }

        let pass_nr = ui.ctx().cumulative_pass_nr();
        let mut escape_consumed = self.escape_consumed_pass == Some(pass_nr);
        if !escape_consumed && ui.is_enabled() && self.handle_escape(ui.ctx()) {
            escape_consumed = true;
        }

        if input.show_section_header {
            paint_section_head(ui, defaults_section_title(input.french));
        }

        let models = catalog::models_for(input.agent_id, input.discovered_models);
        let raw_values = input.defaults.values();
        let shown_values = values_on_listed_row(&models, raw_values.as_ref());
        let chosen_model = shown_values
            .as_ref()
            .and_then(|v| v.get("model"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let model_rows = default_model_rows(&models, chosen_model.as_deref());
        let raw_options: &[Value] = match chosen_model.as_deref() {
            Some(model_id) => catalog::model_options(cat, Some(models.as_slice()), model_id),
            None => &[],
        };
        let launch_options: Vec<&Value> = raw_options
            .iter()
            .filter(|opt| is_launch_option(input.agent_id, opt))
            .collect();
        let has_live_fast = chosen_model.is_some()
            && raw_options
                .iter()
                .any(|opt| is_live_fast_option(input.agent_id, opt));

        let can_probe_ui = cat.probed && input.capabilities.can_refresh_models;
        let avail_w = ui.available_width().max(0.0);

        let mut specs: Vec<ControlSpec> = Vec::with_capacity(2 + launch_options.len());
        specs.push(ControlSpec {
            kind: ControlKind::ModelSelect,
            width: FIELD_WIDTH.min(avail_w),
            height: FIELD_HEIGHT,
        });
        for (idx, opt) in launch_options.iter().enumerate() {
            if opt["kind"]["type"] == "boolean" {
                let label = session_option_label(opt, input.french);
                let text_w = measure_text_width(ui, label, FontId::proportional(11.0));
                let w = (text_w + 6.0 + SWITCH_WIDTH).ceil().min(avail_w);
                specs.push(ControlSpec {
                    kind: ControlKind::BooleanSwitch(idx),
                    width: w,
                    height: CONTROL_HEIGHT,
                });
            } else {
                specs.push(ControlSpec {
                    kind: ControlKind::SelectOption(idx),
                    width: FIELD_WIDTH.min(avail_w),
                    height: FIELD_HEIGHT,
                });
            }
        }
        if can_probe_ui {
            let label = refresh_models_label(input.french);
            let text_w = measure_text_width(ui, label, FontId::proportional(11.5));
            let spinner_extra = if matches!(input.probe, ProbeState::Busy) {
                16.0
            } else {
                0.0
            };
            let w = (text_w + spinner_extra + 20.0).ceil().min(avail_w);
            specs.push(ControlSpec {
                kind: ControlKind::RefreshButton,
                width: w,
                height: CONTROL_HEIGHT,
            });
        }

        let layout = place_flex_wrap(&specs, avail_w);
        let (controls_rect, _) =
            ui.allocate_exact_size(Vec2::new(avail_w, layout.total_height), Sense::hover());

        let mut next_defaults = input.defaults.clone();
        let mut focus_ids = Vec::with_capacity(specs.len());
        let mut refresh_models = false;

        ui.push_id(("agent-model-defaults", input.agent_id), |ui| {
            for (spec, local_rect) in specs.iter().zip(layout.rects.iter()) {
                let item_rect = local_rect.translate(controls_rect.min.to_vec2());
                match spec.kind {
                    ControlKind::ModelSelect => {
                        let own = agent_own_default_label(input.french);
                        let mut select_opts = Vec::with_capacity(1 + model_rows.len());
                        select_opts.push(SelectItem {
                            value: String::new(),
                            label: own.to_owned(),
                        });
                        for m in &model_rows {
                            select_opts.push(SelectItem {
                                value: m.id.clone(),
                                label: m.label.clone(),
                            });
                        }
                        let selected_idx = chosen_model
                            .as_deref()
                            .and_then(|id| {
                                select_opts
                                    .iter()
                                    .position(|it| !it.value.is_empty() && it.value == id)
                            })
                            .unwrap_or(0);
                        let field_label = model_field_label(input.french);
                        let trigger_id = ui.make_persistent_id("model-select");
                        focus_ids.push(trigger_id);
                        if let Some(picked_idx) = self.render_select_field(
                            ui,
                            SelectFieldParams {
                                rect: item_rect,
                                trigger_id,
                                field_label,
                                items: &select_opts,
                                selected_idx,
                            },
                        ) {
                            let picked_val = &select_opts[picked_idx].value;
                            set_default_model(
                                &mut next_defaults,
                                (!picked_val.is_empty()).then(|| picked_val.clone()),
                            );
                        }
                    }
                    ControlKind::SelectOption(opt_idx) => {
                        let opt = launch_options[opt_idx];
                        let opt_id = opt["id"].as_str().unwrap_or("");
                        let field_label = session_option_label(opt, input.french);
                        let def_label = option_default_label(input.french);
                        let choices = opt["kind"]["choices"].as_array();
                        let mut select_opts = Vec::with_capacity(1 + choices.map_or(0, Vec::len));
                        select_opts.push(SelectItem {
                            value: String::new(),
                            label: def_label.to_owned(),
                        });
                        if let Some(choices) = choices {
                            for choice in choices {
                                if let Some(val) = choice["value"].as_str() {
                                    select_opts.push(SelectItem {
                                        value: val.to_owned(),
                                        label: session_choice_label(choice, input.french)
                                            .to_owned(),
                                    });
                                }
                            }
                        }
                        let current_val = shown_values
                            .as_ref()
                            .and_then(|m| m.get(opt_id))
                            .and_then(Value::as_str);
                        let selected_idx = current_val
                            .and_then(|val| {
                                select_opts
                                    .iter()
                                    .position(|it| !it.value.is_empty() && it.value == val)
                            })
                            .unwrap_or(0);
                        let trigger_id = ui.make_persistent_id(("option-select", opt_id));
                        focus_ids.push(trigger_id);
                        if let Some(picked_idx) = self.render_select_field(
                            ui,
                            SelectFieldParams {
                                rect: item_rect,
                                trigger_id,
                                field_label,
                                items: &select_opts,
                                selected_idx,
                            },
                        ) {
                            let picked_val = &select_opts[picked_idx].value;
                            let next_val =
                                (!picked_val.is_empty()).then(|| Value::String(picked_val.clone()));
                            set_default_option(&models, &mut next_defaults, opt_id, next_val);
                        }
                    }
                    ControlKind::BooleanSwitch(opt_idx) => {
                        let opt = launch_options[opt_idx];
                        let opt_id = opt["id"].as_str().unwrap_or("");
                        let label = session_option_label(opt, input.french);
                        // Explicit launch values only: never display an unset boolean as ON
                        // when launch leaves untouched options unset (avoids Vue's CURSOR_THINKING bug).
                        let checked = shown_values.as_ref().and_then(|m| m.get(opt_id))
                            == Some(&Value::Bool(true));
                        let switch_id = ui.make_persistent_id(("option-switch", opt_id));
                        focus_ids.push(switch_id);
                        if render_boolean_switch(ui, item_rect, switch_id, label, checked) {
                            set_default_option(
                                &models,
                                &mut next_defaults,
                                opt_id,
                                Some(Value::Bool(!checked)),
                            );
                        }
                    }
                    ControlKind::RefreshButton => {
                        let busy = matches!(input.probe, ProbeState::Busy);
                        let btn_id = ui.make_persistent_id("refresh-models");
                        focus_ids.push(btn_id);
                        let hint = refresh_models_hint(input.agent_name, input.french);
                        if render_refresh_button(
                            ui,
                            item_rect,
                            btn_id,
                            refresh_models_label(input.french),
                            &hint,
                            busy,
                        ) {
                            refresh_models = true;
                        }
                    }
                }
            }
        });

        ui.add_space(ROW_GAP);
        let status = build_status_text(StatusParams {
            agent_name: input.agent_name,
            catalog_probed: cat.probed,
            models_empty: models.is_empty(),
            has_chosen_model: chosen_model.is_some(),
            has_live_fast,
            listed_meta: input.listed_meta,
            probe: &input.probe,
            capabilities: input.capabilities,
            french: input.french,
        });
        let status_color = if matches!(input.probe, ProbeState::Error(_)) {
            ui.visuals().warn_fg_color
        } else {
            ui.visuals().weak_text_color()
        };
        render_wrapped_status(ui, avail_w, &status, status_color);
        ui.add_space(ROW_GAP);

        Output {
            supported: true,
            changed: (next_defaults != *input.defaults).then_some(next_defaults),
            refresh_models,
            focus_ids,
            escape_consumed,
        }
    }

    fn render_select_field(&mut self, ui: &mut Ui, params: SelectFieldParams<'_>) -> Option<usize> {
        let SelectFieldParams {
            rect,
            trigger_id,
            field_label,
            items,
            selected_idx,
        } = params;
        let label_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HINT_HEIGHT));
        paint_single_line(
            ui,
            label_rect,
            field_label,
            FontId::proportional(11.0),
            ui.visuals().weak_text_color(),
        );
        let trigger_rect = Rect::from_min_size(
            Pos2::new(rect.left(), rect.top() + HINT_HEIGHT + LABEL_GAP),
            Vec2::new(rect.width(), CONTROL_HEIGHT),
        );

        let ctx = ui.ctx().clone();
        let enabled = ui.is_enabled() && !items.is_empty();
        let mut is_open = self
            .open_select
            .as_ref()
            .is_some_and(|open| open.trigger_id == trigger_id);
        let mut chosen: Option<usize> = None;

        if is_open {
            let popup_rect = compute_popup_rect(&ctx, trigger_rect, items.len());
            if let Some(open) = self.open_select.as_mut() {
                open.trigger_rect = trigger_rect;
                open.popup_rect = popup_rect;
                if open.active >= items.len() {
                    open.active = selected_idx.min(items.len().saturating_sub(1));
                }
            }
            let close_from_outside = ctx.input(|i| {
                let press_outside = (i.pointer.button_pressed(PointerButton::Primary)
                    || i.pointer.button_pressed(PointerButton::Secondary))
                    && i.pointer.interact_pos().is_some_and(|pos| {
                        !trigger_rect.contains(pos) && !popup_rect.contains(pos)
                    });
                let scroll_outside = i.raw_scroll_delta != Vec2::ZERO
                    && i.pointer
                        .hover_pos()
                        .is_none_or(|pos| !popup_rect.contains(pos));
                press_outside || scroll_outside
            });
            if close_from_outside {
                self.open_select = None;
                is_open = false;
            }
        }

        let has_focus_before = ui.memory(|m| m.has_focus(trigger_id));
        if enabled && (is_open || has_focus_before) && ctx.input(|i| i.focused) {
            if is_open && ctx.input(|i| i.key_pressed(Key::Tab)) {
                self.open_select = None;
                is_open = false;
            } else {
                let nav_chosen =
                    self.handle_select_keys(&ctx, trigger_id, trigger_rect, items, selected_idx);
                if let Some(idx) = nav_chosen {
                    chosen = Some(idx);
                    self.open_select = None;
                }
                is_open = self
                    .open_select
                    .as_ref()
                    .is_some_and(|open| open.trigger_id == trigger_id);
            }
        }

        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let trigger = ui.interact(trigger_rect, trigger_id, sense);
        let selected_label = items.get(selected_idx).map_or("", |it| it.label.as_str());
        let accessible = format!("{field_label}: {selected_label}");
        trigger.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, enabled, &accessible));

        if enabled && trigger.clicked() {
            trigger.request_focus();
            if is_open {
                self.open_select = None;
                is_open = false;
            } else {
                let popup_rect = compute_popup_rect(&ctx, trigger_rect, items.len());
                self.open_select = Some(OpenSelect {
                    trigger_id,
                    active: selected_idx.min(items.len().saturating_sub(1)),
                    scroll_to_active: true,
                    trigger_rect,
                    popup_rect,
                    search: String::new(),
                    last_search_time: 0.0,
                });
                is_open = true;
            }
        }

        if ui.memory(|m| m.has_focus(trigger_id)) {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    trigger_id,
                    EventFilter {
                        escape: true,
                        vertical_arrows: is_open,
                        ..Default::default()
                    },
                );
            });
        }

        paint_select_trigger(ui, trigger_rect, selected_label, &trigger, is_open, enabled);

        if is_open
            && let Some(picked) =
                self.paint_select_popup(&ctx, trigger_id, trigger_rect, items, selected_idx)
        {
            chosen = Some(picked);
            self.open_select = None;
            ctx.memory_mut(|m| m.request_focus(trigger_id));
        }

        chosen
    }

    fn handle_select_keys(
        &mut self,
        ctx: &egui::Context,
        trigger_id: Id,
        trigger_rect: Rect,
        items: &[SelectItem],
        selected_idx: usize,
    ) -> Option<usize> {
        let is_open = self
            .open_select
            .as_ref()
            .is_some_and(|open| open.trigger_id == trigger_id);

        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            if !is_open {
                self.open_at(ctx, trigger_id, trigger_rect, items, selected_idx);
            } else if let Some(open) = self.open_select.as_mut()
                && open.active + 1 < items.len()
            {
                open.active += 1;
                open.scroll_to_active = true;
            }
            return None;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            if !is_open {
                self.open_at(
                    ctx,
                    trigger_id,
                    trigger_rect,
                    items,
                    items.len().saturating_sub(1),
                );
            } else if let Some(open) = self.open_select.as_mut()
                && open.active > 0
            {
                open.active -= 1;
                open.scroll_to_active = true;
            }
            return None;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Home)) {
            if !is_open {
                self.open_at(ctx, trigger_id, trigger_rect, items, 0);
            } else if let Some(open) = self.open_select.as_mut() {
                open.active = 0;
                open.scroll_to_active = true;
            }
            return None;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::End)) {
            let last = items.len().saturating_sub(1);
            if !is_open {
                self.open_at(ctx, trigger_id, trigger_rect, items, last);
            } else if let Some(open) = self.open_select.as_mut() {
                open.active = last;
                open.scroll_to_active = true;
            }
            return None;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
            || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space))
        {
            if !is_open {
                self.open_at(ctx, trigger_id, trigger_rect, items, selected_idx);
                return None;
            }
            let picked = self.open_select.as_ref().map_or(selected_idx, |open| {
                open.active.min(items.len().saturating_sub(1))
            });
            ctx.memory_mut(|m| m.request_focus(trigger_id));
            return Some(picked);
        }

        let (typed, now) = ctx.input_mut(|i| {
            let mut found = None;
            if !i.modifiers.ctrl && !i.modifiers.alt && !i.modifiers.command {
                i.events.retain(|ev| {
                    if found.is_none()
                        && let egui::Event::Text(text) = ev
                        && text.chars().count() == 1
                    {
                        let ch = text.chars().next().unwrap();
                        if !ch.is_control() && (is_open || ch != ' ') {
                            found = Some(ch);
                            return false;
                        }
                    }
                    true
                });
            }
            (found, i.time)
        });
        if let Some(ch) = typed {
            if !is_open {
                self.open_at(ctx, trigger_id, trigger_rect, items, selected_idx);
            }
            if let Some(open) = self.open_select.as_mut() {
                if now - open.last_search_time > TYPEAHEAD_RESET_SECS {
                    open.search.clear();
                }
                open.last_search_time = now;
                for lower in ch.to_lowercase() {
                    open.search.push(lower);
                }
                let mut chars = open.search.chars();
                let first_char = chars.next();
                let all_same = first_char.is_some_and(|first| chars.all(|c| c == first));
                let needle: String = if all_same {
                    first_char.into_iter().collect()
                } else {
                    open.search.clone()
                };
                let start = if needle.chars().count() == 1 {
                    open.active + 1
                } else {
                    open.active
                };
                for offset in 0..items.len() {
                    let idx = (start + offset) % items.len();
                    if items[idx].label.to_lowercase().starts_with(&needle) {
                        open.active = idx;
                        open.scroll_to_active = true;
                        break;
                    }
                }
            }
        }
        None
    }

    fn open_at(
        &mut self,
        ctx: &egui::Context,
        trigger_id: Id,
        trigger_rect: Rect,
        items: &[SelectItem],
        initial_active: usize,
    ) {
        let popup_rect = compute_popup_rect(ctx, trigger_rect, items.len());
        self.open_select = Some(OpenSelect {
            trigger_id,
            active: initial_active.min(items.len().saturating_sub(1)),
            scroll_to_active: true,
            trigger_rect,
            popup_rect,
            search: String::new(),
            last_search_time: 0.0,
        });
        ctx.memory_mut(|m| m.request_focus(trigger_id));
    }

    fn paint_select_popup(
        &mut self,
        ctx: &egui::Context,
        trigger_id: Id,
        trigger_rect: Rect,
        items: &[SelectItem],
        selected_idx: usize,
    ) -> Option<usize> {
        let popup_rect = compute_popup_rect(ctx, trigger_rect, items.len());
        let pointer_moved = ctx.input(|i| i.pointer.delta() != Vec2::ZERO);
        let (mut active, mut scroll_to_active) = self
            .open_select
            .as_ref()
            .map_or((selected_idx, false), |o| (o.active, o.scroll_to_active));
        let mut chosen = None;

        egui::Area::new(trigger_id.with("ts-popup"))
            .order(Order::Tooltip)
            .fixed_pos(popup_rect.min)
            .interactable(true)
            .show(ctx, |ui| {
                let border_color = border_strong_color(ui);
                let fill = ui.visuals().window_fill;
                egui::Frame::NONE
                    .fill(fill)
                    .stroke(Stroke::new(1.0_f32, border_color))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(POPUP_PADDING_I8))
                    .show(ui, |ui| {
                        let inner_w = (popup_rect.width() - POPUP_PADDING * 2.0).max(24.0);
                        let inner_max_h = (popup_rect.height() - POPUP_PADDING * 2.0).max(20.0);
                        ui.set_min_width(inner_w);
                        ui.set_max_width(inner_w);
                        egui::ScrollArea::vertical()
                            .id_salt(trigger_id.with("ts-scroll"))
                            .max_height(inner_max_h)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                for (idx, item) in items.iter().enumerate() {
                                    let row_w = ui.available_width().max(inner_w);
                                    let (row_rect, _) = ui.allocate_exact_size(
                                        Vec2::new(row_w, POPUP_OPTION_HEIGHT),
                                        Sense::hover(),
                                    );
                                    let opt_resp = ui.interact(
                                        row_rect,
                                        trigger_id.with(("ts-option", idx)),
                                        Sense::click(),
                                    );
                                    let is_selected = idx == selected_idx;
                                    opt_resp.widget_info(|| {
                                        WidgetInfo::selected(
                                            WidgetType::SelectableLabel,
                                            true,
                                            is_selected,
                                            &item.label,
                                        )
                                    });
                                    if pointer_moved && opt_resp.hovered() {
                                        active = idx;
                                    }
                                    if scroll_to_active && idx == active {
                                        opt_resp.scroll_to_me(Some(Align::Center));
                                        scroll_to_active = false;
                                    }
                                    if idx == active {
                                        ui.painter().rect_filled(
                                            row_rect,
                                            CornerRadius::same(4),
                                            ui.visuals().selection.bg_fill,
                                        );
                                    }
                                    let right_pad = if is_selected { 26.0 } else { 8.0 };
                                    let text_rect = Rect::from_min_max(
                                        Pos2::new(row_rect.left() + 8.0, row_rect.top()),
                                        Pos2::new(
                                            (row_rect.right() - right_pad)
                                                .max(row_rect.left() + 8.0),
                                            row_rect.bottom(),
                                        ),
                                    );
                                    paint_single_line(
                                        ui,
                                        text_rect,
                                        &item.label,
                                        FontId::proportional(12.0),
                                        ui.visuals().text_color(),
                                    );
                                    if is_selected {
                                        let check_rect = Rect::from_center_size(
                                            Pos2::new(row_rect.right() - 15.0, row_rect.center().y),
                                            Vec2::splat(14.0),
                                        );
                                        paint_check_icon(
                                            ui.painter(),
                                            check_rect,
                                            ui.visuals().text_color(),
                                        );
                                    }
                                    if opt_resp.clicked() {
                                        chosen = Some(idx);
                                    }
                                }
                            });
                    });
            });

        if let Some(open) = self.open_select.as_mut()
            && open.trigger_id == trigger_id
        {
            open.active = active;
            open.scroll_to_active = scroll_to_active;
            open.popup_rect = popup_rect;
        }
        chosen
    }
}

struct SelectFieldParams<'a> {
    rect: Rect,
    trigger_id: Id,
    field_label: &'a str,
    items: &'a [SelectItem],
    selected_idx: usize,
}

#[derive(Clone, Debug)]
struct SelectItem {
    value: String,
    label: String,
}

#[derive(Clone, Debug)]
struct ModelRow {
    id: String,
    label: String,
}

#[derive(Clone, Copy)]
enum ControlKind {
    ModelSelect,
    SelectOption(usize),
    BooleanSwitch(usize),
    RefreshButton,
}

struct ControlSpec {
    kind: ControlKind,
    width: f32,
    height: f32,
}

struct FlexWrapLayout {
    rects: Vec<Rect>,
    total_height: f32,
}

fn place_flex_wrap(specs: &[ControlSpec], avail_w: f32) -> FlexWrapLayout {
    if specs.is_empty() {
        return FlexWrapLayout {
            rects: Vec::new(),
            total_height: 0.0,
        };
    }
    let mut rects = vec![Rect::NOTHING; specs.len()];
    let mut y = 0.0_f32;
    let mut start = 0usize;
    while start < specs.len() {
        let mut end = start + 1;
        let mut line_w = specs[start].width;
        while end < specs.len() {
            let next_w = line_w + COL_GAP + specs[end].width;
            if next_w <= avail_w + 0.01 {
                line_w = next_w;
                end += 1;
            } else {
                break;
            }
        }
        let line_h = specs[start..end]
            .iter()
            .map(|s| s.height)
            .fold(0.0_f32, f32::max);
        let mut x = 0.0_f32;
        for (offset, spec) in specs[start..end].iter().enumerate() {
            // Matches CSS `.agent-models { align-items: flex-end }`.
            let top = y + (line_h - spec.height);
            rects[start + offset] =
                Rect::from_min_size(Pos2::new(x, top), Vec2::new(spec.width, spec.height));
            x += spec.width + COL_GAP;
        }
        y += line_h;
        if end < specs.len() {
            y += ROW_GAP;
        }
        start = end;
    }
    FlexWrapLayout {
        rects,
        total_height: y,
    }
}

fn compute_popup_rect(ctx: &egui::Context, trigger_rect: Rect, option_count: usize) -> Rect {
    let viewport = ctx.content_rect();
    let below = (viewport.bottom() - trigger_rect.bottom() - 8.0).max(0.0);
    let above = (trigger_rect.top() - viewport.top() - 8.0).max(0.0);
    let open_up = below < POPUP_MIN_WIDTH && above > below;
    let space = if open_up { above } else { below };
    let max_w = (viewport.width() - 16.0).max(40.0);
    let width = trigger_rect.width().max(POPUP_MIN_WIDTH).min(max_w);
    let max_left = (viewport.right() - width - 8.0).max(viewport.left() + 8.0);
    let left = trigger_rect.left().clamp(viewport.left() + 8.0, max_left);
    let natural_h = option_count as f32 * POPUP_OPTION_HEIGHT + POPUP_PADDING * 2.0;
    let max_h = space.clamp(POPUP_OPTION_HEIGHT + POPUP_PADDING * 2.0, POPUP_MAX_HEIGHT);
    let height = natural_h.min(max_h);
    let top = if open_up {
        (trigger_rect.top() - 4.0 - height).max(viewport.top() + 8.0)
    } else {
        trigger_rect.bottom() + 4.0
    };
    Rect::from_min_size(Pos2::new(left, top), Vec2::new(width, height))
}

fn border_strong_color(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        ui.visuals()
            .widgets
            .noninteractive
            .bg_stroke
            .color
            .gamma_multiply(1.35)
    } else {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    }
}

fn paint_section_head(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    let avail_w = ui.available_width().max(0.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail_w, 18.0), Sense::hover());
    paint_single_line(
        ui,
        rect,
        title,
        FontId::proportional(11.5),
        ui.visuals().text_color(),
    );
    ui.add_space(4.0);
}

fn paint_select_trigger(
    ui: &Ui,
    rect: Rect,
    label: &str,
    response: &egui::Response,
    is_open: bool,
    enabled: bool,
) {
    let fill = if ui.visuals().dark_mode {
        ui.visuals().faint_bg_color
    } else {
        ui.visuals().widgets.noninteractive.bg_fill
    };
    let border = if is_open || (enabled && response.hovered()) {
        ui.visuals().selection.stroke.color
    } else {
        border_strong_color(ui)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(8),
            Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
            StrokeKind::Inside,
        );
    }
    let text_color = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().text_color().gamma_multiply(0.5)
    };
    let chevron_color = if enabled {
        ui.visuals().weak_text_color()
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.5)
    };
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + 8.0, rect.top()),
        Pos2::new((rect.right() - 26.0).max(rect.left() + 8.0), rect.bottom()),
    );
    paint_single_line(ui, text_rect, label, FontId::proportional(12.5), text_color);
    let chevron_rect = Rect::from_center_size(
        Pos2::new(
            (rect.right() - 13.0).max(rect.left() + 7.0),
            rect.center().y,
        ),
        Vec2::splat(14.0),
    );
    paint_chevron_down(ui.painter(), chevron_rect, chevron_color);
}

fn render_boolean_switch(ui: &mut Ui, rect: Rect, id: Id, label: &str, checked: bool) -> bool {
    let enabled = ui.is_enabled();
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, checked, label));
    if response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                EventFilter {
                    escape: true,
                    ..Default::default()
                },
            );
        });
    }
    let switch_w = SWITCH_WIDTH.min(rect.width());
    let switch_rect = Rect::from_min_size(
        Pos2::new(
            (rect.right() - switch_w).max(rect.left()),
            rect.center().y - SWITCH_HEIGHT * 0.5,
        ),
        Vec2::new(switch_w, SWITCH_HEIGHT),
    );
    let label_rect = Rect::from_min_max(
        Pos2::new(rect.left(), rect.top()),
        Pos2::new((switch_rect.left() - 6.0).max(rect.left()), rect.bottom()),
    );
    paint_single_line(
        ui,
        label_rect,
        label,
        FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
    );

    // Matches `.set-switch` (`style.css:3920-3962`).
    let pill_fill = if checked {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().selection.bg_fill
    };
    ui.painter()
        .rect_filled(switch_rect, CornerRadius::same(10), pill_fill);
    if !checked {
        ui.painter().rect_stroke(
            switch_rect,
            CornerRadius::same(10),
            Stroke::new(1.0_f32, border_strong_color(ui)),
            StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            switch_rect.expand(2.0),
            CornerRadius::same(12),
            Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
            StrokeKind::Inside,
        );
    }
    let knob_x = if checked {
        (switch_rect.right() - 10.0).max(switch_rect.center().x)
    } else {
        (switch_rect.left() + 10.0).min(switch_rect.center().x)
    };
    let knob_color = if checked {
        Color32::WHITE
    } else {
        Color32::from_rgb(0xae, 0xb4, 0xbf)
    };
    ui.painter()
        .circle_filled(Pos2::new(knob_x, switch_rect.center().y), 7.0, knob_color);

    if enabled && response.clicked() {
        response.request_focus();
        true
    } else {
        false
    }
}

fn render_refresh_button(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    label: &str,
    tooltip: &str,
    busy: bool,
) -> bool {
    let enabled = ui.is_enabled() && !busy;
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.interact(rect, id, sense);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if response.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                EventFilter {
                    escape: true,
                    ..Default::default()
                },
            );
        });
    }
    let fill = if enabled && response.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    let border = if enabled && response.hovered() {
        ui.visuals().selection.stroke.color
    } else {
        border_strong_color(ui)
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0_f32, border),
        StrokeKind::Inside,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(8),
            Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
            StrokeKind::Inside,
        );
    }
    let fg = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let mut text_left = rect.left() + 10.0;
    if busy {
        let spin_rect = Rect::from_center_size(
            Pos2::new((rect.left() + 14.0).min(rect.center().x), rect.center().y),
            Vec2::splat(11.0),
        );
        paint_spinner_arc(ui, spin_rect, fg);
        text_left = (spin_rect.right() + 5.0).min(rect.right());
    }
    let text_rect = Rect::from_min_max(
        Pos2::new(text_left, rect.top()),
        Pos2::new((rect.right() - 8.0).max(text_left), rect.bottom()),
    );
    paint_single_line(ui, text_rect, label, FontId::proportional(11.5), fg);
    let clicked = enabled && response.clicked();
    if clicked {
        response.request_focus();
    }
    let _ = response.on_hover_text(tooltip);
    clicked
}

fn render_wrapped_status(ui: &mut Ui, avail_w: f32, text: &str, color: Color32) {
    if text.is_empty() || avail_w <= 1.0 {
        return;
    }
    let mut job =
        egui::text::LayoutJob::simple(text.to_owned(), FontId::proportional(11.0), color, avail_w);
    job.wrap.break_anywhere = true;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let height = galley.size().y.max(15.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(avail_w, height), Sense::hover());
    ui.painter()
        .with_clip_rect(rect.intersect(ui.clip_rect()))
        .galley(rect.min, galley, color);
}

fn measure_text_width(ui: &mut Ui, text: &str, font: FontId) -> f32 {
    ui.fonts_mut(|f| {
        f.layout_no_wrap(text.to_owned(), font, Color32::WHITE)
            .size()
            .x
    })
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

fn paint_chevron_down(painter: &egui::Painter, rect: Rect, color: Color32) {
    if !rect.is_positive() {
        return;
    }
    let side = rect.width().min(rect.height());
    let fit = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let map = |x: f32, y: f32| {
        Pos2::new(
            fit.left() + (x / 24.0) * side,
            fit.top() + (y / 24.0) * side,
        )
    };
    let stroke = Stroke::new((1.8 * (side / 16.0)).clamp(1.1, 2.2), color);
    painter.add(Shape::line(
        vec![map(6.0, 9.0), map(12.0, 15.0), map(18.0, 9.0)],
        stroke,
    ));
}

fn paint_check_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    if !rect.is_positive() {
        return;
    }
    let side = rect.width().min(rect.height());
    let fit = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let map = |x: f32, y: f32| {
        Pos2::new(
            fit.left() + (x / 24.0) * side,
            fit.top() + (y / 24.0) * side,
        )
    };
    let stroke = Stroke::new((2.0 * (side / 16.0)).clamp(1.2, 2.4), color);
    painter.add(Shape::line(
        vec![map(20.0, 6.0), map(9.0, 17.0), map(4.0, 12.0)],
        stroke,
    ));
}

fn paint_spinner_arc(ui: &Ui, rect: Rect, color: Color32) {
    if !rect.is_positive() {
        return;
    }
    ui.ctx().request_repaint();
    let radius = rect.width().min(rect.height()) * 0.42;
    let base = (ui.input(|i| i.time) * 5.0) as f32;
    let steps = 12usize;
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let angle = base + (i as f32 / steps as f32) * std::f32::consts::TAU * 0.72;
        pts.push(Pos2::new(
            rect.center().x + angle.cos() * radius,
            rect.center().y + angle.sin() * radius,
        ));
    }
    ui.painter()
        .add(Shape::line(pts, Stroke::new(1.5_f32, color)));
}

fn default_model_rows(models: &[Value], chosen_model: Option<&str>) -> Vec<ModelRow> {
    let mut out: Vec<ModelRow> = models
        .iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            let label = m["label"].as_str().unwrap_or(id);
            Some(ModelRow {
                id: id.to_owned(),
                label: label.to_owned(),
            })
        })
        .collect();
    if let Some(chosen) = chosen_model
        && !out.iter().any(|m| m.id == chosen)
    {
        out.push(ModelRow {
            id: chosen.to_owned(),
            label: chosen.to_owned(),
        });
    }
    out
}

/// Applies `AgentModelDefaults.vue:setDefaultModel` (`clearSessionOptionModel` when `None`,
/// preserving per-model `values_by_model`).
pub fn set_default_model(defaults: &mut Defaults, model_id: Option<String>) {
    defaults.model = model_id;
}

/// Applies `AgentModelDefaults.vue:setDefaultOption`, promoting a saved concrete Cursor variant ID
/// onto its grouped base row while preserving decomposed sibling options.
pub fn set_default_option(
    models: &[Value],
    defaults: &mut Defaults,
    option_id: &str,
    value: Option<Value>,
) {
    let raw = defaults.values();
    let Some(shown) = values_on_listed_row(models, raw.as_ref()) else {
        return;
    };
    let Some(shown_model) = shown
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    if defaults.model.as_deref() != Some(shown_model.as_str()) {
        defaults.model = Some(shown_model.clone());
        for (k, v) in &shown {
            if k != "model" {
                defaults.set_option(k, Some(v.clone()));
            }
        }
    }
    defaults.set_option(option_id, value);
}

/// Returns `true` when `option` sets a launch flag or composes into the model ID
/// (`AgentModelDefaults.vue:defaultOptionRows`).
#[must_use]
pub fn is_launch_option(agent: &str, option: &Value) -> bool {
    if option["apply"]["composedIntoModel"] == true {
        return true;
    }
    option["id"] == "effort"
        && matches!(
            agent,
            "claude" | "antigravity" | "amp" | "grok" | "muse" | "copilot" | "pi" | "codex"
        )
}

/// Returns `true` when `option` is a live-session-only Fast Mode toggle (`Claude Code`'s `/fast`).
#[must_use]
pub fn is_live_fast_option(agent: &str, option: &Value) -> bool {
    option["id"] == "fastMode" && !is_launch_option(agent, option)
}

/// Decomposes a concrete Cursor CLI model ID (`"gpt-5.3-codex-high-fast"`) against grouped rows
/// (`cursorModels.js:decomposeCursorModel`).
#[must_use]
pub fn decompose_cursor_model(models: &[Value], id: &str) -> Option<Map<String, Value>> {
    if id.is_empty() {
        return None;
    }
    for model in models {
        let Some(variants) = model["variants"].as_array() else {
            continue;
        };
        let Some(variant) = variants.iter().find(|v| v["id"].as_str() == Some(id)) else {
            continue;
        };
        let row_id = model["id"].as_str()?;
        let mut out = Map::new();
        out.insert("model".into(), Value::String(row_id.to_owned()));
        if model["efforts"]
            .as_array()
            .is_some_and(|efforts| efforts.len() > 1)
            && let Some(effort) = variant["effort"].as_str()
            && !effort.is_empty()
        {
            out.insert("effort".into(), Value::String(effort.to_owned()));
        }
        if model["hasFast"] == true {
            out.insert("fastMode".into(), Value::Bool(variant["fast"] == true));
        }
        if model["hasThinking"] == true {
            out.insert("thinking".into(), Value::Bool(variant["thinking"] == true));
        }
        return Some(out);
    }
    None
}

/// Projects saved values whose `model` is a concrete Cursor variant ID onto its grouped picker row
/// (`agentSessionOptions.js:valuesOnListedRow`).
#[must_use]
pub fn values_on_listed_row(
    models: &[Value],
    values: Option<&Value>,
) -> Option<Map<String, Value>> {
    let obj = values?.as_object()?;
    let model_id = obj.get("model")?.as_str()?;
    if model_id.is_empty() {
        return None;
    }
    if models.iter().any(|m| m["id"].as_str() == Some(model_id)) {
        return Some(obj.clone());
    }
    if let Some(listed) = decompose_cursor_model(models, model_id) {
        let mut merged = obj.clone();
        merged.extend(listed);
        return Some(merged);
    }
    Some(obj.clone())
}

/// Section title `settings.agents.defaultsSection`.
#[must_use]
pub fn defaults_section_title(french: bool) -> &'static str {
    if french {
        "Modèle par défaut"
    } else {
        "Default model"
    }
}

/// Field label `settings.agents.defaultModel`.
#[must_use]
pub fn model_field_label(french: bool) -> &'static str {
    if french { "Modèle" } else { "Model" }
}

/// Unselected model option `pane.sessionOptions.agentDefault`.
#[must_use]
pub fn agent_own_default_label(french: bool) -> &'static str {
    if french {
        "Par défaut de l'agent"
    } else {
        "Agent's own default"
    }
}

/// Unselected select-option choice `pane.sessionOptions.valueIsDefault`.
#[must_use]
pub fn option_default_label(french: bool) -> &'static str {
    if french { "Par défaut" } else { "Default" }
}

/// Refresh button label `settings.agents.refreshModels`.
#[must_use]
pub fn refresh_models_label(french: bool) -> &'static str {
    if french {
        "Actualiser les modèles"
    } else {
        "Refresh models"
    }
}

/// Refresh button tooltip `settings.agents.refreshModelsHint`.
#[must_use]
pub fn refresh_models_hint(agent_name: &str, french: bool) -> String {
    if french {
        format!(
            "Demander à {agent_name} quels modèles il propose (il peut utiliser sa connexion et le réseau ; aucune requête n'est envoyée au modèle)"
        )
    } else {
        format!(
            "Ask {agent_name} which models it offers (it may use its sign-in and the network; no prompt is sent)"
        )
    }
}

/// Localized session-option label (`sessionOptionLabels.js:sessionOptionLabel`).
#[must_use]
pub fn session_option_label(option: &Value, french: bool) -> &str {
    match option["id"].as_str() {
        Some("model") => model_field_label(french),
        Some("effort") => {
            if option["label"].as_str() == Some("Reasoning effort") {
                if french {
                    "Effort de raisonnement"
                } else {
                    "Reasoning effort"
                }
            } else {
                "Effort"
            }
        }
        Some("fastMode") => {
            if french {
                "Mode rapide"
            } else {
                "Fast mode"
            }
        }
        Some("thinking") => {
            if french {
                "Réflexion"
            } else {
                "Thinking"
            }
        }
        _ => option["label"].as_str().unwrap_or(""),
    }
}

/// Localized effort/mode choice label by choice value (`sessionOptionLabels.js:sessionChoiceLabel`).
#[must_use]
pub fn choice_value_label(value: &str, french: bool) -> Option<&'static str> {
    match value {
        "off" => Some(if french { "Désactivé" } else { "Off" }),
        "none" => Some(if french { "Aucune" } else { "None" }),
        "minimal" => Some("Minimal"),
        "low" => Some(if french { "Basse" } else { "Low" }),
        "medium" => Some(if french { "Moyenne" } else { "Medium" }),
        "high" => Some(if french { "Haute" } else { "High" }),
        "xhigh" => Some(if french {
            "Extra élevé"
        } else {
            "Extra high"
        }),
        "max" => Some("Max"),
        "ultra" => Some("Ultra"),
        _ => None,
    }
}

/// Localized choice label for a catalog `{ value, label }` object (`sessionOptionLabels.js:sessionChoiceLabel`).
#[must_use]
pub fn session_choice_label(choice: &Value, french: bool) -> &str {
    if let Some(val) = choice["value"].as_str()
        && let Some(translated) = choice_value_label(val, french)
    {
        return translated;
    }
    choice["label"]
        .as_str()
        .or_else(|| choice["value"].as_str())
        .unwrap_or("")
}

/// Localized seed model description (`sessionOptionLabels.js:modelDescription`).
#[must_use]
pub fn model_description(model: &Value, french: bool) -> &str {
    let Some(desc) = model["description"].as_str() else {
        return "";
    };
    if !french {
        return desc;
    }
    match desc {
        "Most capable for the hardest, longest-running tasks" => {
            "Le plus performant pour les tâches les plus difficiles et les plus longues"
        }
        "Best for everyday, complex tasks" => "Idéal pour les tâches complexes du quotidien",
        "Efficient for routine tasks" => "Efficace pour les tâches courantes",
        "Fastest for quick answers" => "Le plus rapide pour des réponses brèves",
        "Most capable for your hardest and longest-running tasks" => {
            "Le plus performant pour vos tâches les plus difficiles et les plus longues"
        }
        "For your toughest challenges" => "Pour vos défis les plus difficiles",
        "For complex work and everyday tasks" => {
            "Pour le travail complexe et les tâches du quotidien"
        }
        "Most efficient for simpler tasks" => "Le plus efficace pour les tâches simples",
        "xAI's latest frontier model" => "Le dernier modèle de pointe de xAI",
        "xAI's previous frontier model" => "Le précédent modèle de pointe de xAI",
        _ => desc,
    }
}

/// Resolves a model ID to its display name in `models`, or `id` itself when absent
/// (`sessionOptionLabels.js:modelChoiceLabel`).
#[must_use]
pub fn model_choice_label<'a>(models: &'a [Value], id: &'a str) -> &'a str {
    models
        .iter()
        .find(|m| m["id"].as_str() == Some(id))
        .and_then(|m| m["label"].as_str())
        .unwrap_or(id)
}

/// Formats the header pill text (`"Opus · High"`, `"Opus · Extra élevé · Rapide"`, or `"Model"` / `"Modèle"`
/// when unset) matching `sessionOptionLabels.js:sessionPillLabel`.
#[must_use]
pub fn session_pill_label(models: &[Value], values: Option<&Value>, french: bool) -> String {
    let Some(model_id) = values
        .and_then(|v| v.get("model"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return model_field_label(french).to_owned();
    };
    let mut parts = vec![model_choice_label(models, model_id).to_owned()];
    if let Some(effort) = values.and_then(|v| v.get("effort")).and_then(Value::as_str)
        && !effort.is_empty()
    {
        parts.push(
            choice_value_label(effort, french)
                .unwrap_or(effort)
                .to_owned(),
        );
    }
    if values.and_then(|v| v.get("fastMode")) == Some(&Value::Bool(true)) {
        parts.push(if french { "Rapide" } else { "Fast" }.to_owned());
    }
    parts.join(" · ")
}

/// Formats a CLI model-probe failure (`sessionOptionLabels.js:probeErrorText`).
#[must_use]
pub fn probe_error_text(error: &ProbeError<'_>, agent_name: &str, french: bool) -> String {
    match error {
        ProbeError::NotFound => {
            if french {
                format!("{agent_name} est introuvable sur cet ordinateur.")
            } else {
                format!("{agent_name} was not found on this computer.")
            }
        }
        ProbeError::Timeout => {
            if french {
                format!("{agent_name} n'a pas listé ses modèles en 60 s.")
            } else {
                format!("{agent_name} did not list its models within 60 s.")
            }
        }
        ProbeError::TooMuch => {
            if french {
                format!("{agent_name} a renvoyé trop de données de modèles.")
            } else {
                format!("{agent_name} returned too much model data.")
            }
        }
        ProbeError::Empty => {
            if french {
                format!("{agent_name} n'a listé aucun modèle ; la liste intégrée est conservée.")
            } else {
                format!("{agent_name} listed no models; the built-in list stays.")
            }
        }
        ProbeError::Unsupported => {
            if french {
                format!("{agent_name} ne peut pas lister ses modèles.")
            } else {
                format!("{agent_name} cannot list its models.")
            }
        }
        ProbeError::Failed {
            detail: Some(detail),
        } if !detail.is_empty() => {
            if french {
                format!("{agent_name} n'a pas pu lister ses modèles : {detail}")
            } else {
                format!("{agent_name} could not list its models: {detail}")
            }
        }
        ProbeError::Failed { .. } => {
            if french {
                format!("{agent_name} n'a pas pu lister ses modèles.")
            } else {
                format!("{agent_name} could not list its models.")
            }
        }
    }
}

struct StatusParams<'a> {
    agent_name: &'a str,
    catalog_probed: bool,
    models_empty: bool,
    has_chosen_model: bool,
    has_live_fast: bool,
    listed_meta: Option<ListedMeta<'a>>,
    probe: &'a ProbeState<'a>,
    capabilities: Capabilities,
    french: bool,
}

fn build_status_text(p: StatusParams<'_>) -> String {
    let base = match p.probe {
        ProbeState::Busy => {
            if p.french {
                format!("Demande de ses modèles à {}…", p.agent_name)
            } else {
                format!("Asking {} for its models…", p.agent_name)
            }
        }
        ProbeState::Error(err) => probe_error_text(err, p.agent_name, p.french),
        ProbeState::Idle => {
            if let Some(meta) = p.listed_meta {
                if p.french {
                    format!(
                        "{} {} par {} le {}.",
                        meta.count,
                        if meta.count <= 1 {
                            "modèle listé"
                        } else {
                            "modèles listés"
                        },
                        p.agent_name,
                        meta.date_label
                    )
                } else {
                    format!(
                        "{} {} listed by {} on {}.",
                        meta.count,
                        if meta.count == 1 { "model" } else { "models" },
                        p.agent_name,
                        meta.date_label
                    )
                }
            } else if p.catalog_probed && p.capabilities.can_refresh_models {
                if p.models_empty {
                    if p.french {
                        format!(
                            "Pas encore de liste. Actualiser demande à {} les modèles qu'il propose.",
                            p.agent_name
                        )
                    } else {
                        format!(
                            "No list yet. Refresh asks {} for the models it offers.",
                            p.agent_name
                        )
                    }
                } else if p.french {
                    format!(
                        "Liste intégrée. Actualiser demande à {} les modèles de votre compte.",
                        p.agent_name
                    )
                } else {
                    format!(
                        "Built-in list. Refresh asks {} for the models your account has.",
                        p.agent_name
                    )
                }
            } else if p.models_empty {
                if p.french {
                    format!("Aucune liste de modèles intégrée pour {}.", p.agent_name)
                } else {
                    format!("No built-in model list for {}.", p.agent_name)
                }
            } else if p.french {
                "Liste de modèles intégrée. L’agent peut proposer d’autres modèles dans ses propres réglages."
                    .to_owned()
            } else {
                "Built-in model list. The agent may also offer other models in its own settings."
                    .to_owned()
            }
        }
    };

    let mut parts = vec![base];
    if p.has_live_fast && p.capabilities.running_pane_model_menu {
        parts.push(if p.french {
            "Le mode rapide se change dans un panneau en cours d'exécution (menu du panneau > Modèle)."
                .to_owned()
        } else {
            "Fast mode is switched in a running pane (pane menu > Model).".to_owned()
        });
    }
    if p.has_chosen_model {
        parts.push(if p.capabilities.running_pane_restart_badge {
            if p.french {
                "S'applique aux panneaux que vous démarrez désormais ; ceux en cours affichent Redémarrer pour appliquer."
                    .to_owned()
            } else {
                "Applies to panes you start from now on; running ones show Restart to apply."
                    .to_owned()
            }
        } else if p.french {
            "S'applique aux panneaux que vous démarrez désormais.".to_owned()
        } else {
            "Applies to panes you start from now on.".to_owned()
        });
    }
    parts.join(" ")
}
