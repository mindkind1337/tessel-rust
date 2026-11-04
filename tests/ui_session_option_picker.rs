//! Standalone headless `egui` integration tests for `src/session_option_picker_ui.rs`
//! (`SessionOptionPicker.vue`, `.pane-model-chip`, `.pane-apply`, `PaneActionsMenu.vue`
//! model entry, EN/FR locales, capability gating, disabled `ui` / `pending` / `pane_busy` /
//! `user_typing` protection, Unicode UTF-8 safety, `4px` viewport clamping across all 7 dark
//! themes + light mode, keyboard navigation, and 1-step `Escape` isolation).

#[test]
fn native_chat_picker_explains_application_boundary_and_hides_terminal_options() {
    let defaults = std::collections::BTreeMap::new();
    for (agent, phrase) in [
        ("codex", "next Codex turn"),
        ("claude", "next Claude connection"),
    ] {
        let ctx = egui::Context::default();
        let mut picker = session_option_picker_ui::SessionOptionPickerUi::default();
        let mut input = session_option_picker_ui::SessionPickerInput::new(agent, &defaults);
        input.native_chat = true;
        input.capabilities.can_open_picker = true;
        let mut result = session_option_picker_ui::SessionPickerOutput::default();
        let full = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = picker.show_inline(ui, &input);
            });
        });
        assert!(result.supported && !result.disabled);
        assert!(result.note.contains(phrase));
        let text = painted_texts(&full)
            .into_iter()
            .map(|v| v.0)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("Choose in agent picker"));
        assert!(!text.contains("/fast"));
    }
}

#[allow(dead_code)]
#[path = "../src/agent_env.rs"]
mod agent_env;

#[allow(dead_code)]
#[path = "../src/agent_arg_options.rs"]
mod agent_arg_options;

#[allow(dead_code)]
#[path = "../src/agent_model_antigravity.rs"]
mod agent_model_antigravity;

#[allow(dead_code)]
#[path = "../src/agent_model_catalog.rs"]
mod agent_model_catalog;

#[allow(dead_code)]
mod workspace {
    use serde::{Deserialize, Serialize};
    use std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
    };

    pub type Result<T> = std::result::Result<T, String>;

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    pub struct Workspace {
        #[serde(default)]
        pub agent_model_defaults: BTreeMap<String, crate::agent_model_defaults::Defaults>,
    }

    pub struct Store {
        file: PathBuf,
    }

    impl Store {
        pub fn open(dir: &Path) -> Result<(Self, Workspace)> {
            let file = dir.join("workspace.json");
            let state = match fs::read(&file) {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                Err(_) => Workspace::default(),
            };
            Ok((Self { file }, state))
        }

        pub fn save(&self, state: &Workspace) -> Result<()> {
            let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
            fs::write(&self.file, bytes).map_err(|e| e.to_string())
        }
    }
}

#[allow(dead_code)]
#[path = "../src/agent_model_defaults.rs"]
mod agent_model_defaults;

#[allow(dead_code)]
#[path = "../src/agent_model_launch.rs"]
mod agent_model_launch;

#[allow(dead_code)]
#[path = "../src/theme_palettes.rs"]
mod theme_palettes;

#[allow(dead_code)]
#[path = "../src/theme.rs"]
mod theme;

#[allow(dead_code)]
#[path = "../src/agent_model_defaults_ui.rs"]
mod agent_model_defaults_ui;

#[allow(dead_code)]
#[path = "../src/pane_session_model.rs"]
mod pane_session_model;

#[path = "../src/session_option_picker_ui.rs"]
mod session_option_picker_ui;

use agent_model_defaults::Defaults;
use eframe::egui::{
    self, Color32, Context, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape,
    Vec2,
};
use pane_session_model::{
    AgentPermissionMode, CommandDelivery, ModelPickOutcome, ModelSource, PaneModelNode,
    PanePickContext, ShownAgentModel, apply_restart_title, effective_agent_launch,
    launch_session_values, launch_signature,
};
use serde_json::{Value, json};
use session_option_picker_ui::{
    CHIP_ANCHOR_OFFSET_Y, HEADER_CHIP_HEIGHT, HEADER_MODEL_CHIP_HIDE_MAX_WIDTH,
    HEADER_MODEL_CHIP_MAX_WIDTH, HeaderModelChipInput, HeaderModelChipOutput, OptionRowMode,
    PICKER_MAX_HEIGHT, PICKER_MAX_WIDTH, PICKER_MIN_WIDTH, PICKER_VIEWPORT_MARGIN,
    PaneMenuModelEntry, PaneMenuModelInput, PickerCloseReason, RestartApplyBadgeInput,
    RestartApplyBadgeOutput, SessionOptionPickerUi, SessionPickerCapabilities, SessionPickerEvent,
    SessionPickerInput, SessionPickerOutput, capability_gated_menu_note,
    choose_in_agent_picker_label, has_model_choice, option_row_mode, pane_menu_model_hint,
    pane_menu_model_label, plan_picker_event, resolve_pane_menu_model_entry,
    show_header_model_chip, show_restart_apply_badge, toggle_option_label, typed_line_reason,
};
use std::collections::BTreeMap;

fn collect_texts(shape: &Shape, out: &mut Vec<(String, Rect, Color32)>) {
    match shape {
        Shape::Text(t) => {
            out.push((
                t.galley.job.text.clone(),
                Rect::from_min_size(t.pos, t.galley.size()),
                t.galley
                    .job
                    .sections
                    .first()
                    .map_or(t.fallback_color, |s| s.format.color),
            ));
        }
        Shape::Vec(list) => {
            for inner in list {
                collect_texts(inner, out);
            }
        }
        _ => {}
    }
}

fn painted_texts(output: &egui::FullOutput) -> Vec<(String, Rect, Color32)> {
    let mut out = Vec::new();
    for clipped in &output.shapes {
        collect_texts(&clipped.shape, &mut out);
    }
    out
}

fn painted_strings(output: &egui::FullOutput) -> Vec<String> {
    painted_texts(output)
        .into_iter()
        .map(|(text, _, _)| text)
        .collect()
}

fn find_text_rect(output: &egui::FullOutput, target: &str) -> Option<Rect> {
    painted_texts(output)
        .into_iter()
        .find(|(text, _, _)| text == target)
        .map(|(_, rect, _)| rect)
}

fn collect_galley_rows(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Text(t) => {
            for row in &t.galley.rows {
                let s: String = row.glyphs.iter().map(|g| g.chr).collect();
                out.push(s);
            }
        }
        Shape::Vec(list) => {
            for inner in list {
                collect_galley_rows(inner, out);
            }
        }
        _ => {}
    }
}

fn painted_visual_lines(output: &egui::FullOutput) -> Vec<String> {
    let mut out = Vec::new();
    for clipped in &output.shapes {
        collect_galley_rows(&clipped.shape, &mut out);
    }
    out
}

fn key_event(key: Key, pressed: bool, repeat: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers: Modifiers::NONE,
    }
}

fn pointer_down_events(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]
}

fn pointer_up_events(pos: Pos2) -> Vec<Event> {
    vec![Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]
}

fn make_launched_node(
    agent_id: &str,
    session_options: Option<Value>,
    persisted_defaults: &BTreeMap<String, Defaults>,
    discovered: Option<&[Value]>,
) -> PaneModelNode {
    let models = agent_model_catalog::models_for(agent_id, discovered);
    let vals = launch_session_values(session_options.as_ref(), persisted_defaults, agent_id);
    let spec = effective_agent_launch(
        agent_id,
        agent_id,
        None,
        AgentPermissionMode::default(),
        vals.as_ref(),
        Some(&models),
    );
    PaneModelNode {
        agent_id: agent_id.to_owned(),
        agent_command: Some(agent_id.to_owned()),
        detected: false,
        chat_view: false,
        remote_host_id: None,
        session_options,
        launch_sig: Some(launch_signature(&spec)),
        model_choice: None,
    }
}

fn run_inline_frame(
    ctx: &Context,
    picker: &mut SessionOptionPickerUi,
    input: &SessionPickerInput<'_>,
    viewport: Vec2,
    events: Vec<Event>,
) -> (egui::FullOutput, SessionPickerOutput) {
    run_inline_frame_enabled(ctx, picker, input, viewport, true, events)
}

fn run_inline_frame_enabled(
    ctx: &Context,
    picker: &mut SessionOptionPickerUi,
    input: &SessionPickerInput<'_>,
    viewport: Vec2,
    enabled: bool,
    events: Vec<Event>,
) -> (egui::FullOutput, SessionPickerOutput) {
    let mut picker_out = SessionPickerOutput::default();
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
        events,
        focused: true,
        ..Default::default()
    };
    let full = ctx.run(raw, |ctx| {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    picker_out = picker.show_inline(ui, input);
                });
            });
    });
    (full, picker_out)
}

struct HeaderAndPopoverFrame {
    full: egui::FullOutput,
    chip: HeaderModelChipOutput,
    badge: RestartApplyBadgeOutput,
    picker: SessionPickerOutput,
}

struct HeaderHarness<'a> {
    pane_header_width: f32,
    viewport: Vec2,
    enabled: bool,
    is_agent: bool,
    detected: bool,
    agent_id: &'a str,
    pane_running: bool,
    pane_busy: bool,
    pending: bool,
    user_typing: bool,
    guest_runs: bool,
    guest_model: Option<&'a str>,
    launch_stale: bool,
    session_options: Option<&'a Value>,
    discovered_models: Option<&'a [Value]>,
    agent_model: Option<&'a ShownAgentModel>,
    persisted_defaults: &'a BTreeMap<String, Defaults>,
    capabilities: SessionPickerCapabilities,
    french: bool,
}

impl<'a> HeaderHarness<'a> {
    fn new(agent_id: &'a str, persisted_defaults: &'a BTreeMap<String, Defaults>) -> Self {
        Self {
            pane_header_width: 520.0,
            viewport: Vec2::new(800.0, 600.0),
            enabled: true,
            is_agent: true,
            detected: false,
            agent_id,
            pane_running: true,
            pane_busy: false,
            pending: false,
            user_typing: false,
            guest_runs: false,
            guest_model: None,
            launch_stale: false,
            session_options: None,
            discovered_models: None,
            agent_model: None,
            persisted_defaults,
            capabilities: SessionPickerCapabilities::default(),
            french: false,
        }
    }
}

fn step_header_and_popover(
    ctx: &Context,
    picker: &mut SessionOptionPickerUi,
    harness: &HeaderHarness<'_>,
    events: Vec<Event>,
) -> HeaderAndPopoverFrame {
    let mut chip_out = HeaderModelChipOutput::default();
    let mut badge_out = RestartApplyBadgeOutput::default();
    let mut picker_out = SessionPickerOutput::default();
    let pane_id = Id::new(("pane-slot", harness.agent_id));

    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, harness.viewport)),
        events,
        focused: true,
        ..Default::default()
    };
    let full = ctx.run(raw, |ctx| {
        if harness.enabled {
            picker.handle_escape(ctx);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                ui.add_enabled_ui(harness.enabled, |ui| {
                    ui.horizontal(|ui| {
                        chip_out = show_header_model_chip(
                            ui,
                            pane_id,
                            &HeaderModelChipInput {
                                pane_header_width: harness.pane_header_width,
                                is_agent: harness.is_agent,
                                detected: harness.detected,
                                agent_id: harness.agent_id,
                                guest_runs: harness.guest_runs,
                                guest_model: harness.guest_model,
                                agent_model: harness.agent_model,
                                capabilities: harness.capabilities,
                                french: harness.french,
                            },
                        );
                        badge_out = show_restart_apply_badge(
                            ui,
                            pane_id,
                            RestartApplyBadgeInput {
                                launch_stale: harness.launch_stale,
                                capabilities: harness.capabilities,
                                french: harness.french,
                            },
                        );
                    });
                });
                if chip_out.clicked
                    && let Some(rect) = chip_out.rect
                {
                    picker.toggle_at_chip(ctx, rect, chip_out.id);
                }
            });

        let mut input = SessionPickerInput::new(harness.agent_id, harness.persisted_defaults);
        input.is_agent = harness.is_agent;
        input.detected = harness.detected;
        input.pane_running = harness.pane_running;
        input.pane_busy = harness.pane_busy;
        input.pending = harness.pending;
        input.user_typing = harness.user_typing;
        input.session_options = harness.session_options;
        input.discovered_models = harness.discovered_models;
        input.agent_model = harness.agent_model;
        input.capabilities = harness.capabilities;
        input.french = harness.french;
        input.trigger_rect = chip_out.rect;
        picker_out = picker.show_popover(ctx, pane_id, &input);
    });

    HeaderAndPopoverFrame {
        full,
        chip: chip_out,
        badge: badge_out,
        picker: picker_out,
    }
}

/// Runs two neutral frames after opening an `egui::Area` popover so `egui`'s first-frame
/// sizing pass completes and all text galleys are painted in the returned `FullOutput`.
fn open_and_stabilize_at_chip(
    ctx: &Context,
    picker: &mut SessionOptionPickerUi,
    harness: &HeaderHarness<'_>,
) -> HeaderAndPopoverFrame {
    let f0 = step_header_and_popover(ctx, picker, harness, vec![]);
    picker.open_at_chip(ctx, f0.chip.rect.unwrap(), f0.chip.id);
    step_header_and_popover(ctx, picker, harness, vec![]);
    step_header_and_popover(ctx, picker, harness, vec![])
}

struct ParentModalEnv<'a> {
    ctx: Context,
    picker: SessionOptionPickerUi,
    defaults: &'a BTreeMap<String, Defaults>,
    shown: &'a ShownAgentModel,
    session_options: Option<&'a Value>,
    viewport: Vec2,
    parent_modal_closed: bool,
}

fn step_parent_modal(
    env: &mut ParentModalEnv<'_>,
    events: Vec<Event>,
) -> (SessionPickerOutput, Option<Id>) {
    let mut chip_id = None;
    let mut picker_out = SessionPickerOutput::default();
    let pane_id = Id::new("esc-pane");
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, env.viewport)),
        events,
        focused: true,
        ..Default::default()
    };
    let _ = env.ctx.run(raw, |ctx| {
        let esc_taken = env.picker.handle_escape(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let chip = show_header_model_chip(
                    ui,
                    pane_id,
                    &HeaderModelChipInput {
                        pane_header_width: 520.0,
                        is_agent: true,
                        detected: false,
                        agent_id: "claude",
                        guest_runs: false,
                        guest_model: None,
                        agent_model: Some(env.shown),
                        capabilities: SessionPickerCapabilities::all_live(),
                        french: false,
                    },
                );
                chip_id = chip.id;
                if chip.clicked
                    && let Some(r) = chip.rect
                {
                    env.picker.toggle_at_chip(ctx, r, chip.id);
                }
                let mut p_in = SessionPickerInput::new("claude", env.defaults);
                p_in.pane_running = true;
                p_in.session_options = env.session_options;
                p_in.agent_model = Some(env.shown);
                p_in.capabilities = SessionPickerCapabilities::all_live();
                p_in.trigger_rect = chip.rect;
                picker_out = env.picker.show_popover(ctx, pane_id, &p_in);
            });
        if !esc_taken && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            env.parent_modal_closed = true;
        }
    });
    (picker_out, chip_id)
}

#[test]
fn en_and_fr_labels_menu_entries_and_capability_notes_match_vue() {
    for id in [
        "claude",
        "codex",
        "gemini",
        "opencode",
        "cursor",
        "antigravity",
        "copilot",
        "kimi",
        "grok",
        "amp",
        "pi",
        "muse",
    ] {
        assert!(has_model_choice(true, false, id), "{id} should have choice");
        assert!(!has_model_choice(true, true, id), "detected pane excluded");
        assert!(!has_model_choice(false, false, id), "shell pane excluded");
    }
    assert!(!has_model_choice(true, false, "aider"));
    assert!(!has_model_choice(true, false, "goose"));
    assert!(!has_model_choice(true, false, "unknown-agent"));

    // Exact strings from `SessionOptionPicker.vue:116` & `i18n/locales/fr/panes.json:769-770`.
    assert_eq!(
        choose_in_agent_picker_label(false),
        "Choose in agent picker…"
    );
    assert_eq!(
        choose_in_agent_picker_label(true),
        "Choisir dans le sélecteur d'agents…"
    );
    assert_eq!(pane_menu_model_label(false), "Model…");
    assert_eq!(pane_menu_model_label(true), "Modèle…");
    assert_eq!(
        pane_menu_model_hint(false),
        "Choose the model this agent uses"
    );
    assert_eq!(
        pane_menu_model_hint(true),
        "Choisir le modèle utilisé par cet agent"
    );
    assert_eq!(
        typed_line_reason(false),
        "A line is typed in its terminal: send or clear it there first."
    );
    assert_eq!(
        typed_line_reason(true),
        "Une ligne est tapée dans son terminal : envoyez-la ou effacez-la d'abord."
    );

    let claude_cat = agent_model_catalog::find("claude").unwrap();
    let opus_opts = agent_model_catalog::model_options(claude_cat, None, "opus");
    let fast_opt = opus_opts.iter().find(|o| o["id"] == "fastMode").unwrap();
    let effort_opt = opus_opts.iter().find(|o| o["id"] == "effort").unwrap();
    assert_eq!(toggle_option_label(fast_opt, false), "Toggle fast mode");
    assert_eq!(toggle_option_label(fast_opt, true), "Basculer mode rapide");

    assert_eq!(
        option_row_mode(
            "claude",
            fast_opt,
            false,
            SessionPickerCapabilities::all_live()
        ),
        OptionRowMode::Hidden
    );
    assert_eq!(
        option_row_mode(
            "claude",
            fast_opt,
            true,
            SessionPickerCapabilities::restart_only()
        ),
        OptionRowMode::Hidden
    );
    assert_eq!(
        option_row_mode(
            "claude",
            fast_opt,
            true,
            SessionPickerCapabilities::all_live()
        ),
        OptionRowMode::Toggle
    );
    assert_eq!(
        option_row_mode(
            "claude",
            effort_opt,
            true,
            SessionPickerCapabilities::all_live()
        ),
        OptionRowMode::Select
    );

    let codex_cat = agent_model_catalog::find("codex").unwrap();
    let codex_effort = agent_model_catalog::model_options(codex_cat, None, "gpt-5.6-sol")
        .iter()
        .find(|o| o["id"] == "effort")
        .unwrap();
    assert_eq!(
        option_row_mode(
            "codex",
            codex_effort,
            true,
            SessionPickerCapabilities::restart_only()
        ),
        OptionRowMode::Select
    );
    assert_eq!(
        option_row_mode(
            "codex",
            codex_effort,
            true,
            SessionPickerCapabilities::all_live()
        ),
        OptionRowMode::AgentPicker
    );

    let cursor_cat = agent_model_catalog::find("cursor").unwrap();
    let cursor_thinking = agent_model_catalog::model_options(cursor_cat, None, "claude-opus-4-8")
        .iter()
        .find(|o| o["id"] == "thinking")
        .unwrap();
    assert_eq!(
        option_row_mode(
            "cursor",
            cursor_thinking,
            false,
            SessionPickerCapabilities::default()
        ),
        OptionRowMode::Switch
    );

    // Capability-honest notes never claim live switching when `live_command_switch` is off.
    assert_eq!(
        capability_gated_menu_note(
            "claude",
            true,
            SessionPickerCapabilities::restart_only(),
            false
        ),
        "A model picked here applies when the agent restarts."
    );
    assert_eq!(
        capability_gated_menu_note(
            "cursor",
            true,
            SessionPickerCapabilities::restart_only(),
            true
        ),
        "Un modèle choisi ici s'applique au redémarrage de l'agent."
    );
    assert_eq!(
        capability_gated_menu_note("claude", true, SessionPickerCapabilities::all_live(), false),
        ""
    );
    assert_eq!(
        capability_gated_menu_note("cursor", true, SessionPickerCapabilities::all_live(), true),
        "Un modèle choisi ici s'applique aussitôt ; son effort ou Fast ouvre son propre sélecteur dans le terminal (Tab sur le modèle)."
    );

    // `PaneActionsMenu.vue` model entry resolution (None vs ReadOnlyFact vs InteractiveItem).
    let defaults = BTreeMap::new();
    assert_eq!(
        resolve_pane_menu_model_entry(&PaneMenuModelInput {
            is_agent: false,
            detected: false,
            agent_id: "",
            session_options: None,
            discovered_models: None,
            agent_model: None,
            persisted_defaults: &defaults,
            capabilities: SessionPickerCapabilities::all_live(),
            french: false,
        }),
        PaneMenuModelEntry::None
    );

    let shown = ShownAgentModel {
        model: "sonnet".into(),
        name: Some("Sonnet".into()),
        effort: Some("high".into()),
        chosen_effort: None,
        source: ModelSource::Session,
        at: None,
    };
    let readonly = resolve_pane_menu_model_entry(&PaneMenuModelInput {
        is_agent: true,
        detected: false,
        agent_id: "claude",
        session_options: None,
        discovered_models: None,
        agent_model: Some(&shown),
        persisted_defaults: &defaults,
        capabilities: SessionPickerCapabilities::default(),
        french: true,
    });
    assert_eq!(
        readonly,
        PaneMenuModelEntry::ReadOnlyFact {
            label: "Modèle".into(),
            value: "Sonnet · Élevé".into(),
            tooltip: "Modèle : sonnet (raisonnement Élevé)\nd'après sa dernière réponse".into(),
        }
    );

    let manual_opts = json!({ "model": "opus", "effort": "max" });
    let interactive = resolve_pane_menu_model_entry(&PaneMenuModelInput {
        is_agent: true,
        detected: false,
        agent_id: "claude",
        session_options: Some(&manual_opts),
        discovered_models: None,
        agent_model: None,
        persisted_defaults: &defaults,
        capabilities: SessionPickerCapabilities::restart_only(),
        french: false,
    });
    assert_eq!(
        interactive,
        PaneMenuModelEntry::InteractiveItem {
            label: "Model…".into(),
            shortcut: "Opus · Max".into(),
            tooltip: "Choose the model this agent uses".into(),
            is_manual_override: true,
        }
    );
}

#[test]
fn default_and_restart_only_capabilities_never_advertise_unwired_commands() {
    let ctx = Context::default();
    theme::apply(&ctx, true, None);
    let defaults = BTreeMap::new();
    let prefs = BTreeMap::new();
    let shown_opus = ShownAgentModel {
        model: "claude-opus-5-5".into(),
        name: Some("Opus".into()),
        effort: None,
        chosen_effort: None,
        source: ModelSource::Session,
        at: None,
    };

    // 1. All-false default capabilities: chip is static, restart badge stays hidden, popover refuses to open,
    //    and `plan_picker_event` emits zero `typed_commands` for both `Set` and `Action`.
    let mut picker = SessionOptionPickerUi::new();
    let mut harness = HeaderHarness::new("claude", &defaults);
    harness.agent_model = Some(&shown_opus);
    harness.launch_stale = true;
    harness.capabilities = SessionPickerCapabilities::default();

    let f0 = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    assert!(f0.chip.visible);
    assert!(!f0.chip.interactive);
    assert_eq!(f0.chip.aria_label, "Model: Opus");
    let chip_rect0 = f0.chip.rect.unwrap();
    assert!((chip_rect0.height() - HEADER_CHIP_HEIGHT).abs() < 0.01);
    assert!(chip_rect0.width() <= HEADER_MODEL_CHIP_MAX_WIDTH);
    assert!(
        !f0.badge.visible,
        "unwired restart badge must remain hidden"
    );
    assert!(!picker.is_open());

    let chip_center = chip_rect0.center();
    step_header_and_popover(
        &ctx,
        &mut picker,
        &harness,
        pointer_down_events(chip_center),
    );
    let f_click =
        step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(chip_center));
    assert!(!f_click.chip.clicked);
    assert!(!picker.is_open());

    // Even if a caller explicitly calls `open_at_point`, default capabilities close immediately.
    picker.open_at_point(&ctx, Pos2::new(40.0, 40.0), None);
    let f_blocked = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    assert!(!picker.is_open());
    assert!(f_blocked.picker.popup_rect.is_none());

    let node = make_launched_node("claude", Some(json!({ "model": "opus" })), &defaults, None);
    let pick_ctx = PanePickContext {
        is_agent: true,
        pane_running: true,
        pane_busy: false,
        pending: false,
        user_typing: false,
        chat_view_available: true,
        discovered_models: None,
        agent_model: Some(&shown_opus),
        agent_prefs: &prefs,
        persisted_defaults: &defaults,
        permissions: AgentPermissionMode::default(),
        now_ms: 1_000,
        french: false,
    };

    let all_false_set_model = plan_picker_event(
        &mut picker,
        &SessionPickerEvent::Set {
            option_id: "model".into(),
            value: Some(json!("sonnet")),
        },
        &node,
        &pick_ctx,
        SessionPickerCapabilities::default(),
        None,
    );
    assert_eq!(
        all_false_set_model.plan.outcome,
        Some(ModelPickOutcome::Saved)
    );
    assert!(all_false_set_model.plan.typed_commands.is_empty());

    let all_false_set_effort = plan_picker_event(
        &mut picker,
        &SessionPickerEvent::Set {
            option_id: "effort".into(),
            value: Some(json!("max")),
        },
        &node,
        &pick_ctx,
        SessionPickerCapabilities::default(),
        None,
    );
    assert_eq!(
        all_false_set_effort.plan.outcome,
        Some(ModelPickOutcome::Saved)
    );
    assert!(all_false_set_effort.plan.typed_commands.is_empty());

    let all_false_action = plan_picker_event(
        &mut picker,
        &SessionPickerEvent::Action {
            option_id: "fastMode".into(),
        },
        &node,
        &pick_ctx,
        SessionPickerCapabilities::default(),
        None,
    );
    assert_eq!(all_false_action.plan.outcome, None);
    assert!(all_false_action.plan.typed_commands.is_empty());

    // 2. `restart_only()` capabilities: opens popover, omits `/fast` and `"Choose in agent picker…"`,
    //    saves picks cleanly with zero PTY commands, keeps popover open, and lights `"Restart to apply"`.
    harness.capabilities = SessionPickerCapabilities::restart_only();
    harness.launch_stale = false;
    let f_ready = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    assert!(f_ready.chip.interactive);
    assert_eq!(f_ready.chip.aria_label, "Model: Opus. Choose the model");

    let chip_rect = f_ready.chip.rect.unwrap();
    step_header_and_popover(
        &ctx,
        &mut picker,
        &harness,
        pointer_down_events(chip_rect.center()),
    );
    let f_click_open = step_header_and_popover(
        &ctx,
        &mut picker,
        &harness,
        pointer_up_events(chip_rect.center()),
    );
    assert!(f_click_open.chip.clicked);
    assert!(picker.is_open());
    assert_eq!(
        picker.anchor_pos(),
        Some(Pos2::new(
            chip_rect.left(),
            chip_rect.bottom() + CHIP_ANCHOR_OFFSET_Y
        ))
    );
    let f_open = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);

    let texts = painted_strings(&f_open.full);
    assert!(
        !texts.iter().any(|t| t == "Toggle fast mode"),
        "`/fast` must not be advertised when live_toggle_command is false"
    );
    assert_eq!(
        f_open.picker.note,
        "A model picked here applies when the agent restarts."
    );

    let sonnet_pos = find_text_rect(&f_open.full, "Sonnet").unwrap().center();
    step_header_and_popover(&ctx, &mut picker, &harness, pointer_down_events(sonnet_pos));
    let f_pick =
        step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(sonnet_pos));
    let ev = f_pick.picker.event.expect("Sonnet click should emit Set");
    assert_eq!(
        ev,
        SessionPickerEvent::Set {
            option_id: "model".into(),
            value: Some(json!("sonnet")),
        }
    );

    let planned = plan_picker_event(
        &mut picker,
        &ev,
        &node,
        &pick_ctx,
        harness.capabilities,
        None,
    );
    assert_eq!(planned.plan.outcome, Some(ModelPickOutcome::Saved));
    assert!(planned.plan.typed_commands.is_empty());
    assert!(planned.plan.launch_stale_after);
    assert!(!planned.closed_picker, "Saved picks keep popover open");
    assert!(picker.is_open());

    // Verify `close_with_reason(ActionCompleted)` and French `"Redémarrer pour appliquer"` badge.
    assert!(PickerCloseReason::ActionCompleted.refocus_pane());
    picker.close_with_reason(&ctx, PickerCloseReason::ActionCompleted);
    harness.french = true;
    harness.launch_stale = planned.plan.launch_stale_after;
    let f_badge = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    assert_eq!(
        f_badge.picker.closed,
        Some(PickerCloseReason::ActionCompleted)
    );
    assert!(f_badge.badge.visible);
    assert!(f_badge.badge.id.is_some());
    assert_eq!(f_badge.badge.text, "Redémarrer pour appliquer");
    assert_eq!(f_badge.badge.tooltip, apply_restart_title(true));
    let badge_pos = f_badge.badge.rect.unwrap().center();
    step_header_and_popover(&ctx, &mut picker, &harness, pointer_down_events(badge_pos));
    let f_badge_click =
        step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(badge_pos));
    assert!(f_badge_click.badge.clicked);
}

#[test]
fn claude_live_picker_en_fr_unicode_aliases_synthetic_row_and_observer_bridge() {
    let ctx = Context::default();
    theme::apply(&ctx, true, None);

    let mut defaults = BTreeMap::new();
    defaults.insert(
        "claude".into(),
        Defaults {
            model: Some("haiku".into()),
            ..Default::default()
        },
    );
    let prefs = BTreeMap::new();
    let shown_1m = ShownAgentModel {
        model: "claude-opus-5-5[1m]".into(),
        name: Some("Claude Opus 5.5 (1M)".into()),
        effort: Some("high".into()),
        chosen_effort: None,
        source: ModelSource::Session,
        at: None,
    };
    let saved_1m = json!({ "model": "opus[1m]" });
    let unlisted = json!({ "model": "claude-custom-preview" });
    let unicode_emoji_mid = json!({ "model": "😀a" });
    let unicode_emoji_end = json!({ "model": "modèle😀" });
    let unicode_1m = json!({ "model": "😀[1M]" });

    let mut picker = SessionOptionPickerUi::new();
    let mut input = SessionPickerInput::new("claude", &defaults);
    input.pane_running = true;
    input.agent_model = Some(&shown_1m);
    input.capabilities = SessionPickerCapabilities::all_live();
    input.french = true;
    input.default_hint = Some("Suit les paramètres globaux");

    let (full_fr, out_fr) =
        run_inline_frame(&ctx, &mut picker, &input, Vec2::new(420.0, 640.0), vec![]);
    assert!(out_fr.supported);
    assert!(!out_fr.disabled);
    assert_eq!(out_fr.chosen_model, None);
    assert_eq!(out_fr.options_model.as_deref(), Some("opus"));
    assert_eq!(out_fr.note, "");

    let texts_fr = painted_strings(&full_fr);
    assert!(texts_fr.iter().any(|t| t == "Modèle"));
    assert!(
        texts_fr
            .iter()
            .any(|t| t == "Par défaut des paramètres (Haiku)")
    );
    assert!(
        texts_fr
            .iter()
            .any(|t| t.contains("Suit les paramètres globaux"))
    );
    assert!(
        texts_fr
            .iter()
            .any(|t| t.contains("Idéal pour les tâches complexes du quotidien"))
    );
    assert!(texts_fr.iter().any(|t| t == "Basculer mode rapide"));
    assert!(texts_fr.iter().any(|t| t == "Effort"));

    // Explicit `session_options` with `"opus[1m]"` matches `"opus"` without a duplicate synthetic row.
    input.session_options = Some(&saved_1m);
    let (full_alias, out_alias) =
        run_inline_frame(&ctx, &mut picker, &input, Vec2::new(420.0, 640.0), vec![]);
    assert_eq!(out_alias.chosen_model.as_deref(), Some("opus"));
    let opus_count = painted_strings(&full_alias)
        .iter()
        .filter(|t| t.as_str() == "Opus")
        .count();
    assert_eq!(
        opus_count, 1,
        "alias must highlight Opus without duplicating"
    );

    // Unicode model names whose `len - 4` lands mid-codepoint (`"😀a"`, len=5 -> index 1) never panic.
    for (val, expected_id) in [
        (&unicode_emoji_mid, "😀a"),
        (&unicode_emoji_end, "modèle😀"),
        (&unicode_1m, "😀[1M]"),
    ] {
        input.session_options = Some(val);
        let (_, u_out) =
            run_inline_frame(&ctx, &mut picker, &input, Vec2::new(420.0, 640.0), vec![]);
        assert_eq!(u_out.chosen_model.as_deref(), Some(expected_id));
    }

    // Unlisted saved model renders a synthetic checked row at the bottom of the model section.
    input.session_options = Some(&unlisted);
    input.french = false;
    let (full_unlisted, out_unlisted) =
        run_inline_frame(&ctx, &mut picker, &input, Vec2::new(420.0, 640.0), vec![]);
    assert_eq!(
        out_unlisted.chosen_model.as_deref(),
        Some("claude-custom-preview")
    );
    assert!(
        painted_strings(&full_unlisted)
            .iter()
            .any(|t| t == "claude-custom-preview")
    );

    // Live Claude `/model sonnet`, `/effort max`, and `/fast` planner bridge.
    let mut pop_picker = SessionOptionPickerUi::new();
    let mut harness = HeaderHarness::new("claude", &defaults);
    // Pin Opus for this live scenario: the inherited Haiku setting above has no effort option.
    let live_values = json!({ "model": "opus" });
    harness.session_options = Some(&live_values);
    harness.agent_model = Some(&shown_1m);
    harness.capabilities = SessionPickerCapabilities::all_live();
    let f_open = open_and_stabilize_at_chip(&ctx, &mut pop_picker, &harness);
    assert!(pop_picker.is_open());

    let node = make_launched_node("claude", Some(live_values.clone()), &defaults, None);
    let pick_ctx = PanePickContext {
        is_agent: true,
        pane_running: true,
        pane_busy: false,
        pending: false,
        user_typing: false,
        chat_view_available: true,
        discovered_models: None,
        agent_model: Some(&shown_1m),
        agent_prefs: &prefs,
        persisted_defaults: &defaults,
        permissions: AgentPermissionMode::default(),
        now_ms: 2_500,
        french: false,
    };

    // `/model sonnet` -> AwaitClaudeConfirmation, holds default-model file, closes popover + refocuses.
    let sonnet_pos = find_text_rect(&f_open.full, "Sonnet").unwrap().center();
    step_header_and_popover(
        &ctx,
        &mut pop_picker,
        &harness,
        pointer_down_events(sonnet_pos),
    );
    let f_sonnet = step_header_and_popover(
        &ctx,
        &mut pop_picker,
        &harness,
        pointer_up_events(sonnet_pos),
    );
    let sonnet_res = plan_picker_event(
        &mut pop_picker,
        &f_sonnet.picker.event.unwrap(),
        &node,
        &pick_ctx,
        harness.capabilities,
        None,
    );
    assert_eq!(
        sonnet_res.plan.outcome,
        Some(ModelPickOutcome::AwaitClaudeConfirmation)
    );
    assert!(sonnet_res.plan.hold_claude_default_model);
    assert_eq!(sonnet_res.plan.typed_commands.len(), 1);
    assert_eq!(sonnet_res.plan.typed_commands[0].command, "/model sonnet");
    assert_eq!(
        sonnet_res.plan.typed_commands[0].delivery,
        CommandDelivery::Write
    );
    assert!(sonnet_res.closed_picker);
    assert!(sonnet_res.refocus_pane);
    assert!(!pop_picker.is_open());

    // Reopen and pick `Max` effort -> `/effort max`, adopts launch_sig (not stale), closes popover.
    let f_open2 = open_and_stabilize_at_chip(&ctx, &mut pop_picker, &harness);
    let max_pos = find_text_rect(&f_open2.full, "Max").unwrap().center();
    step_header_and_popover(
        &ctx,
        &mut pop_picker,
        &harness,
        pointer_down_events(max_pos),
    );
    let f_max =
        step_header_and_popover(&ctx, &mut pop_picker, &harness, pointer_up_events(max_pos));
    let max_res = plan_picker_event(
        &mut pop_picker,
        f_max.picker.event.as_ref().unwrap(),
        &node,
        &pick_ctx,
        harness.capabilities,
        None,
    );
    assert_eq!(max_res.plan.outcome, Some(ModelPickOutcome::Sent));
    assert_eq!(max_res.plan.typed_commands[0].command, "/effort max");
    assert!(!max_res.plan.launch_stale_after);
    assert!(max_res.closed_picker);
    assert!(max_res.refocus_pane);

    // Reopen and click `"Toggle fast mode"` -> `/fast`, ActionSent, closes popover.
    open_and_stabilize_at_chip(&ctx, &mut pop_picker, &harness);
    // The last action is below the scroll viewport; navigate to it before activating.
    step_header_and_popover(
        &ctx,
        &mut pop_picker,
        &harness,
        vec![key_event(Key::End, true, false)],
    );
    let f_fast = step_header_and_popover(
        &ctx,
        &mut pop_picker,
        &harness,
        vec![key_event(Key::Enter, true, false)],
    );
    assert_eq!(
        f_fast.picker.event,
        Some(SessionPickerEvent::Action {
            option_id: "fastMode".into()
        })
    );
    let fast_res = plan_picker_event(
        &mut pop_picker,
        &f_fast.picker.event.unwrap(),
        &node,
        &pick_ctx,
        harness.capabilities,
        None,
    );
    assert_eq!(fast_res.plan.outcome, Some(ModelPickOutcome::ActionSent));
    assert_eq!(fast_res.plan.typed_commands[0].command, "/fast");
    assert!(fast_res.closed_picker);
    assert!(fast_res.refocus_pane);
}

#[test]
fn codex_live_picker_keeps_menu_open_on_saved_pick_and_closes_on_agent_picker_action() {
    let ctx = Context::default();
    theme::apply(&ctx, true, None);
    let defaults = BTreeMap::new();
    let prefs = BTreeMap::new();
    let shown = ShownAgentModel {
        model: "gpt-5.6-sol".into(),
        name: Some("GPT-5.6 Sol".into()),
        effort: Some("high".into()),
        chosen_effort: None,
        source: ModelSource::Session,
        at: None,
    };

    let mut picker = SessionOptionPickerUi::new();
    let mut harness = HeaderHarness::new("codex", &defaults);
    harness.agent_model = Some(&shown);
    harness.capabilities = SessionPickerCapabilities::all_live();

    // Verify French note wraps at word boundaries (never splitting `"l'agent."` into `"l'agen"` + `"t."`).
    harness.french = true;
    let f_fr = open_and_stabilize_at_chip(&ctx, &mut picker, &harness);
    let visual_rows = painted_visual_lines(&f_fr.full);
    assert!(
        !visual_rows
            .iter()
            .any(|row| row.trim_end().ends_with("l'agen")),
        "French note must wrap at spaces, got rows: {visual_rows:?}"
    );
    picker.close();

    harness.french = false;
    let f_open = open_and_stabilize_at_chip(&ctx, &mut picker, &harness);
    let picker_action_count = painted_strings(&f_open.full)
        .iter()
        .filter(|t| t.as_str() == "Choose in agent picker…")
        .count();
    assert_eq!(
        picker_action_count, 2,
        "Running Codex renders Choose in agent picker… for both Model and Reasoning effort"
    );

    let initial_node = make_launched_node("codex", None, &defaults, None);
    let pick_ctx = PanePickContext {
        is_agent: true,
        pane_running: true,
        pane_busy: false,
        pending: false,
        user_typing: false,
        chat_view_available: true,
        discovered_models: None,
        agent_model: Some(&shown),
        agent_prefs: &prefs,
        persisted_defaults: &defaults,
        permissions: AgentPermissionMode::default(),
        now_ms: 5_000,
        french: false,
    };

    // Picking a Codex model row while running saves for restart and keeps `.pane-model-menu` open!
    let codex_pos = find_text_rect(&f_open.full, "GPT-5.2 Codex")
        .unwrap()
        .center();
    step_header_and_popover(&ctx, &mut picker, &harness, pointer_down_events(codex_pos));
    let f_codex =
        step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(codex_pos));
    let saved_res = plan_picker_event(
        &mut picker,
        &f_codex.picker.event.unwrap(),
        &initial_node,
        &pick_ctx,
        harness.capabilities,
        None,
    );
    assert_eq!(saved_res.plan.outcome, Some(ModelPickOutcome::Saved));
    assert!(saved_res.plan.typed_commands.is_empty());
    assert!(saved_res.plan.launch_stale_after);
    assert!(!saved_res.closed_picker);
    assert!(!saved_res.refocus_pane);
    assert!(picker.is_open());
    let saved_node = saved_res.plan.node;

    // Picking `"Agent's own default"` resets `session_options` to `None` and clears stale badge.
    harness.session_options = saved_node.session_options.as_ref();
    let f_updated = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    let def_pos = find_text_rect(&f_updated.full, "Agent's own default")
        .unwrap()
        .center();
    step_header_and_popover(&ctx, &mut picker, &harness, pointer_down_events(def_pos));
    let f_def = step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(def_pos));
    let reset_res = plan_picker_event(
        &mut picker,
        &f_def.picker.event.unwrap(),
        &saved_node,
        &pick_ctx,
        harness.capabilities,
        None,
    );
    assert_eq!(reset_res.plan.node.session_options, None);
    assert!(!reset_res.plan.launch_stale_after);
    assert!(picker.is_open());

    // Clicking `"Choose in agent picker…"` types `/model` (`CommandDelivery::Type`) and closes popover.
    harness.session_options = None;
    let f_ready = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    let action_pos = find_text_rect(&f_ready.full, "Choose in agent picker…")
        .unwrap()
        .center();
    step_header_and_popover(&ctx, &mut picker, &harness, pointer_down_events(action_pos));
    let f_act = step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(action_pos));
    let act_res = plan_picker_event(
        &mut picker,
        &f_act.picker.event.unwrap(),
        &reset_res.plan.node,
        &pick_ctx,
        harness.capabilities,
        None,
    );
    assert_eq!(act_res.plan.outcome, Some(ModelPickOutcome::ActionSent));
    assert_eq!(act_res.plan.typed_commands.len(), 1);
    assert_eq!(act_res.plan.typed_commands[0].command, "/model");
    assert_eq!(
        act_res.plan.typed_commands[0].delivery,
        CommandDelivery::Type
    );
    assert!(act_res.closed_picker);
    assert!(act_res.refocus_pane);
    assert!(!picker.is_open());
}

#[test]
fn cursor_grouped_variants_and_seed_boolean_switches_match_vue_parity() {
    let ctx = Context::default();
    theme::apply(&ctx, true, None);
    let defaults = BTreeMap::new();
    let prefs = BTreeMap::new();

    // 1. Seed Cursor model (`claude-opus-4-8`, no `variants` array): unset `thinking` starts unchecked (`false`),
    //    so clicking it toggles to `Some(Bool(true))`.
    let mut seed_picker = SessionOptionPickerUi::new();
    let seed_chosen = json!({ "model": "claude-opus-4-8" });
    let mut seed_input = SessionPickerInput::new("cursor", &defaults);
    seed_input.session_options = Some(&seed_chosen);
    seed_input.capabilities = SessionPickerCapabilities::all_live();

    let (seed_full, _) = run_inline_frame(
        &ctx,
        &mut seed_picker,
        &seed_input,
        Vec2::new(380.0, 560.0),
        vec![],
    );
    let thinking_pos = find_text_rect(&seed_full, "Thinking").unwrap().center();
    run_inline_frame(
        &ctx,
        &mut seed_picker,
        &seed_input,
        Vec2::new(380.0, 560.0),
        pointer_down_events(thinking_pos),
    );
    let (_, seed_click_out) = run_inline_frame(
        &ctx,
        &mut seed_picker,
        &seed_input,
        Vec2::new(380.0, 560.0),
        pointer_up_events(thinking_pos),
    );
    assert_eq!(
        seed_click_out.event,
        Some(SessionPickerEvent::Set {
            option_id: "thinking".into(),
            value: Some(Value::Bool(true)),
        }),
        "Seed Cursor model must start with Thinking unchecked when unset"
    );

    // 2. CLI-discovered grouped Cursor models (`cursorModels.js` shape with variant objects):
    //    decomposes `"gpt-5.3-codex-high-fast"` onto base row `"gpt-5.3-codex"` with `Fast mode` ON (`true`),
    //    and grouped `"claude-opus-5"` (`defaultValue: true`) starts with `Thinking` ON (`true`).
    let discovered = vec![
        json!({
            "id": "gpt-5.3-codex",
            "label": "GPT-5.3 Codex",
            "efforts": ["low", "high"],
            "defaultEffort": "low",
            "hasFast": true,
            "hasThinking": false,
            "variants": [
                { "id": "gpt-5.3-codex", "effort": "low", "fast": false, "thinking": false },
                { "id": "gpt-5.3-codex-high", "effort": "high", "fast": false, "thinking": false },
                { "id": "gpt-5.3-codex-fast", "effort": "low", "fast": true, "thinking": false },
                { "id": "gpt-5.3-codex-high-fast", "effort": "high", "fast": true, "thinking": false }
            ],
            "options": [
                {
                    "id": "effort",
                    "label": "Effort",
                    "kind": {
                        "type": "select",
                        "choices": [
                            { "value": "low", "label": "Low" },
                            { "value": "high", "label": "High" }
                        ]
                    },
                    "apply": { "composedIntoModel": true }
                },
                {
                    "id": "fastMode",
                    "label": "Fast mode",
                    "kind": { "type": "boolean", "defaultValue": false },
                    "apply": { "composedIntoModel": true }
                }
            ]
        }),
        json!({
            "id": "claude-opus-5",
            "label": "Claude 5 Opus",
            "efforts": ["high", "max"],
            "defaultEffort": "high",
            "hasFast": false,
            "hasThinking": true,
            "defaultThinking": true,
            "variants": [
                { "id": "claude-opus-5", "effort": "high", "fast": false, "thinking": false },
                { "id": "claude-opus-5-thinking", "effort": "high", "fast": false, "thinking": true },
                { "id": "claude-opus-5-thinking-max", "effort": "max", "fast": false, "thinking": true }
            ],
            "options": [
                {
                    "id": "thinking",
                    "label": "Thinking",
                    "kind": { "type": "boolean", "defaultValue": true },
                    "apply": { "composedIntoModel": true }
                },
                {
                    "id": "effort",
                    "label": "Effort",
                    "kind": {
                        "type": "select",
                        "choices": [
                            { "value": "high", "label": "High" },
                            { "value": "max", "label": "Max" }
                        ]
                    },
                    "apply": { "composedIntoModel": true }
                }
            ]
        }),
    ];

    let shown_variant = ShownAgentModel {
        model: "gpt-5.3-codex-high-fast".into(),
        name: None,
        effort: None,
        chosen_effort: None,
        source: ModelSource::Screen,
        at: None,
    };
    let mut grp_picker = SessionOptionPickerUi::new();
    let mut grp_input = SessionPickerInput::new("cursor", &defaults);
    grp_input.pane_running = true;
    grp_input.discovered_models = Some(&discovered);
    grp_input.agent_model = Some(&shown_variant);
    grp_input.capabilities = SessionPickerCapabilities::all_live();

    let (grp_full, grp_out) = run_inline_frame(
        &ctx,
        &mut grp_picker,
        &grp_input,
        Vec2::new(400.0, 560.0),
        vec![],
    );
    assert_eq!(grp_out.chosen_model.as_deref(), Some("gpt-5.3-codex"));
    let grp_texts = painted_strings(&grp_full);
    assert!(!grp_texts.iter().any(|t| t == "gpt-5.3-codex-high-fast"));
    assert!(grp_texts.iter().any(|t| t == "GPT-5.3 Codex"));
    assert!(grp_texts.iter().any(|t| t == "Fast mode"));

    // Clicking `Fast mode` toggles from `true` -> `false`.
    let fast_pos = find_text_rect(&grp_full, "Fast mode").unwrap().center();
    run_inline_frame(
        &ctx,
        &mut grp_picker,
        &grp_input,
        Vec2::new(400.0, 560.0),
        pointer_down_events(fast_pos),
    );
    let (_, fast_toggle_out) = run_inline_frame(
        &ctx,
        &mut grp_picker,
        &grp_input,
        Vec2::new(400.0, 560.0),
        pointer_up_events(fast_pos),
    );
    assert_eq!(
        fast_toggle_out.event,
        Some(SessionPickerEvent::Set {
            option_id: "fastMode".into(),
            value: Some(Value::Bool(false)),
        })
    );

    // Grouped `"claude-opus-5"` (`defaultValue: true` on `thinking`) starts checked (`true`),
    // so clicking `Thinking` toggles from `true` -> `false`.
    let opus5_chosen = json!({ "model": "claude-opus-5" });
    grp_input.session_options = Some(&opus5_chosen);
    grp_input.pane_running = false;
    let (opus5_full, _) = run_inline_frame(
        &ctx,
        &mut grp_picker,
        &grp_input,
        Vec2::new(400.0, 560.0),
        vec![],
    );
    let grp_thinking_pos = find_text_rect(&opus5_full, "Thinking").unwrap().center();
    run_inline_frame(
        &ctx,
        &mut grp_picker,
        &grp_input,
        Vec2::new(400.0, 560.0),
        pointer_down_events(grp_thinking_pos),
    );
    let (_, grp_thinking_out) = run_inline_frame(
        &ctx,
        &mut grp_picker,
        &grp_input,
        Vec2::new(400.0, 560.0),
        pointer_up_events(grp_thinking_pos),
    );
    assert_eq!(
        grp_thinking_out.event,
        Some(SessionPickerEvent::Set {
            option_id: "thinking".into(),
            value: Some(Value::Bool(false)),
        })
    );

    // Live Cursor effort pick opens `/model` TUI picker (`ModelPickOutcome::Picker`) and exits `chat_view`.
    let mut chat_node = make_launched_node("cursor", None, &defaults, Some(&discovered));
    chat_node.chat_view = true;
    let pick_ctx = PanePickContext {
        is_agent: true,
        pane_running: true,
        pane_busy: false,
        pending: false,
        user_typing: false,
        chat_view_available: true,
        discovered_models: Some(&discovered),
        agent_model: Some(&shown_variant),
        agent_prefs: &prefs,
        persisted_defaults: &defaults,
        permissions: AgentPermissionMode::default(),
        now_ms: 8_000,
        french: false,
    };
    let effort_res = plan_picker_event(
        &mut grp_picker,
        &SessionPickerEvent::Set {
            option_id: "effort".into(),
            value: Some(json!("high")),
        },
        &chat_node,
        &pick_ctx,
        SessionPickerCapabilities::all_live(),
        None,
    );
    assert_eq!(effort_res.plan.outcome, Some(ModelPickOutcome::Picker));
    assert_eq!(effort_res.plan.typed_commands[0].command, "/model");
    assert!(!effort_res.plan.node.chat_view);
    assert!(effort_res.closed_picker);
    assert!(effort_res.refocus_pane);
}

#[test]
fn disabled_parent_ui_pending_busy_and_user_typing_block_interaction_and_commands() {
    let ctx = Context::default();
    theme::apply(&ctx, true, None);
    let defaults = BTreeMap::new();
    let prefs = BTreeMap::new();
    let shown = ShownAgentModel {
        model: "claude-opus-5-5".into(),
        name: Some("Opus".into()),
        effort: None,
        chosen_effort: None,
        source: ModelSource::Session,
        at: None,
    };

    let mut picker = SessionOptionPickerUi::new();
    let mut input = SessionPickerInput::new("claude", &defaults);
    input.pane_running = true;
    input.agent_model = Some(&shown);
    input.capabilities = SessionPickerCapabilities::all_live();

    // 1. Parent `ui` disabled (`enabled = false`): rows & header controls are inert and `focus_ids` empty.
    let (dis_ui_full, dis_ui_out) = run_inline_frame_enabled(
        &ctx,
        &mut picker,
        &input,
        Vec2::new(400.0, 560.0),
        false,
        vec![],
    );
    assert!(dis_ui_out.disabled);
    assert!(dis_ui_out.focus_ids.is_empty());
    let sonnet_dis_pos = find_text_rect(&dis_ui_full, "Sonnet").unwrap().center();
    run_inline_frame_enabled(
        &ctx,
        &mut picker,
        &input,
        Vec2::new(400.0, 560.0),
        false,
        pointer_down_events(sonnet_dis_pos),
    );
    let (_, dis_ui_click) = run_inline_frame_enabled(
        &ctx,
        &mut picker,
        &input,
        Vec2::new(400.0, 560.0),
        false,
        pointer_up_events(sonnet_dis_pos),
    );
    assert!(dis_ui_click.event.is_none());

    let mut dis_harness = HeaderHarness::new("claude", &defaults);
    dis_harness.enabled = false;
    dis_harness.launch_stale = true;
    dis_harness.agent_model = Some(&shown);
    dis_harness.capabilities = SessionPickerCapabilities::all_live();
    let f_dis_hdr = step_header_and_popover(&ctx, &mut picker, &dis_harness, vec![]);
    let dis_chip_pos = f_dis_hdr.chip.rect.unwrap().center();
    step_header_and_popover(
        &ctx,
        &mut picker,
        &dis_harness,
        pointer_down_events(dis_chip_pos),
    );
    let f_dis_hdr_click = step_header_and_popover(
        &ctx,
        &mut picker,
        &dis_harness,
        pointer_up_events(dis_chip_pos),
    );
    assert!(!f_dis_hdr_click.chip.clicked);
    assert!(!picker.is_open());

    // 2. `pending == true`: picker is disabled and clicks emit `None`.
    input.pending = true;
    let (pend_full, pend_out) =
        run_inline_frame(&ctx, &mut picker, &input, Vec2::new(400.0, 560.0), vec![]);
    assert!(pend_out.disabled);
    assert!(pend_out.focus_ids.is_empty());
    let pend_sonnet = find_text_rect(&pend_full, "Sonnet").unwrap().center();
    run_inline_frame(
        &ctx,
        &mut picker,
        &input,
        Vec2::new(400.0, 560.0),
        pointer_down_events(pend_sonnet),
    );
    let (_, pend_click) = run_inline_frame(
        &ctx,
        &mut picker,
        &input,
        Vec2::new(400.0, 560.0),
        pointer_up_events(pend_sonnet),
    );
    assert!(pend_click.event.is_none());
    input.pending = false;

    // 3. `pane_busy == true`: disables all rows, clears `focus_ids`, and ignores clicks & `Enter`.
    input.pane_busy = true;
    input.french = true;
    let (busy_full, busy_out) =
        run_inline_frame(&ctx, &mut picker, &input, Vec2::new(400.0, 560.0), vec![]);
    assert!(busy_out.disabled);
    assert!(busy_out.focus_ids.is_empty());
    assert_eq!(
        busy_out.disabled_reason,
        "Il travaille : son modèle pourra changer une fois qu'il sera inactif."
    );
    let sonnet_pos = find_text_rect(&busy_full, "Sonnet").unwrap().center();
    run_inline_frame(
        &ctx,
        &mut picker,
        &input,
        Vec2::new(400.0, 560.0),
        pointer_down_events(sonnet_pos),
    );
    let (_, busy_click_out) = run_inline_frame(
        &ctx,
        &mut picker,
        &input,
        Vec2::new(400.0, 560.0),
        vec![
            Event::PointerButton {
                pos: sonnet_pos,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            },
            key_event(Key::Enter, true, false),
        ],
    );
    assert!(busy_click_out.event.is_none());

    // Custom `disabled_reason_override` takes precedence cleanly when set.
    input.disabled_reason_override = Some("Verrou personnalisé");
    let (_, override_out) =
        run_inline_frame(&ctx, &mut picker, &input, Vec2::new(400.0, 560.0), vec![]);
    assert!(override_out.disabled);
    assert_eq!(override_out.disabled_reason, "Verrou personnalisé");
    input.disabled_reason_override = None;

    // 4. `user_typing == true` disables UI rows AND blocks `plan_picker_event` from emitting slash commands
    //    while preserving accurate `launch_stale_after` on a node whose effort already differs from launch.
    input.pane_busy = false;
    input.user_typing = true;
    input.french = false;
    let (_, typing_out) =
        run_inline_frame(&ctx, &mut picker, &input, Vec2::new(400.0, 560.0), vec![]);
    assert!(typing_out.disabled);
    assert_eq!(typing_out.disabled_reason, typed_line_reason(false));

    let mut stale_node =
        make_launched_node("claude", Some(json!({ "model": "opus" })), &defaults, None);
    stale_node.session_options = Some(json!({ "model": "opus", "effort": "max" }));
    let typing_ctx = PanePickContext {
        is_agent: true,
        pane_running: true,
        pane_busy: false,
        pending: false,
        user_typing: true,
        chat_view_available: true,
        discovered_models: None,
        agent_model: Some(&shown),
        agent_prefs: &prefs,
        persisted_defaults: &defaults,
        permissions: AgentPermissionMode::default(),
        now_ms: 9_000,
        french: false,
    };
    let blocked_set = plan_picker_event(
        &mut picker,
        &SessionPickerEvent::Set {
            option_id: "model".into(),
            value: Some(json!("sonnet")),
        },
        &stale_node,
        &typing_ctx,
        SessionPickerCapabilities::all_live(),
        None,
    );
    assert_eq!(blocked_set.plan.outcome, Some(ModelPickOutcome::Busy));
    assert!(blocked_set.plan.typed_commands.is_empty());
    assert_eq!(
        blocked_set.plan.toast.as_ref().map(|t| t.text.as_str()),
        Some(typed_line_reason(false))
    );
    assert!(
        blocked_set.plan.launch_stale_after,
        "stale effort difference must be preserved when user_typing blocks a live pick"
    );
    assert_eq!(blocked_set.plan.node, stale_node);
    assert!(!blocked_set.closed_picker);

    let blocked_act = plan_picker_event(
        &mut picker,
        &SessionPickerEvent::Action {
            option_id: "fastMode".into(),
        },
        &stale_node,
        &typing_ctx,
        SessionPickerCapabilities::all_live(),
        None,
    );
    assert!(blocked_act.plan.typed_commands.is_empty());
    assert_eq!(
        blocked_act.plan.toast.as_ref().map(|t| t.text.as_str()),
        Some(typed_line_reason(false))
    );
    assert!(!blocked_act.closed_picker);
}

#[test]
fn header_chip_breakpoint_viewport_clamping_all_themes_keyboard_and_escape_isolation() {
    let ctx = Context::default();
    let defaults = BTreeMap::new();
    let shown = ShownAgentModel {
        model: "claude-opus-5-5".into(),
        name: Some("Opus".into()),
        effort: Some("high".into()),
        chosen_effort: None,
        source: ModelSource::Session,
        at: None,
    };

    // 1. `<= 340px` responsive hide & Live Share `guest_runs` static chip.
    theme::apply(&ctx, true, None);
    let mut picker = SessionOptionPickerUi::new();
    let mut harness = HeaderHarness::new("claude", &defaults);
    harness.agent_model = Some(&shown);
    harness.capabilities = SessionPickerCapabilities::all_live();

    harness.pane_header_width = HEADER_MODEL_CHIP_HIDE_MAX_WIDTH;
    let f_narrow = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    assert!(!f_narrow.chip.visible);

    harness.pane_header_width = HEADER_MODEL_CHIP_HIDE_MAX_WIDTH + 1.0;
    let f_wide = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    assert!(f_wide.chip.visible);
    assert!(f_wide.chip.interactive);

    harness.guest_runs = true;
    harness.guest_model = Some("Sonnet");
    let f_guest = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    assert!(f_guest.chip.visible);
    assert!(!f_guest.chip.interactive);
    assert_eq!(f_guest.chip.text, "Sonnet");
    assert_eq!(f_guest.chip.aria_label, "Model: Sonnet");
    assert!(f_guest.chip.tooltip.is_empty());
    harness.guest_runs = false;
    harness.guest_model = None;
    harness.pane_header_width = 520.0;

    // 2. Bottom-right `4px` viewport clamping across all 7 dark themes + light mode.
    let mut theme_configs: Vec<(bool, Option<theme_palettes::ThemeId>)> = theme_palettes::THEMES
        .iter()
        .copied()
        .map(|id| (true, Some(id)))
        .collect();
    theme_configs.push((false, None));

    for (dark, theme_id) in theme_configs {
        theme::apply(&ctx, dark, theme_id);
        let mut themed_picker = SessionOptionPickerUi::new();
        themed_picker.open_at_point(&ctx, Pos2::new(780.0, 580.0), None);
        step_header_and_popover(&ctx, &mut themed_picker, &harness, vec![]);
        let frame = step_header_and_popover(&ctx, &mut themed_picker, &harness, vec![]);
        let popup = frame
            .picker
            .popup_rect
            .expect("popover must paint in every theme");
        assert_eq!(themed_picker.last_popup_rect(), Some(popup));
        assert!(popup.left() >= PICKER_VIEWPORT_MARGIN - 0.01);
        assert!(popup.top() >= PICKER_VIEWPORT_MARGIN - 0.01);
        assert!(popup.right() <= harness.viewport.x - PICKER_VIEWPORT_MARGIN + 0.01);
        assert!(popup.bottom() <= harness.viewport.y - PICKER_VIEWPORT_MARGIN + 0.01);
        assert!(popup.width() >= PICKER_MIN_WIDTH);
        assert!(popup.width() <= PICKER_MAX_WIDTH + 8.0 + 0.01);
        assert!(popup.height() <= (harness.viewport.y * 0.70).min(PICKER_MAX_HEIGHT) + 8.0 + 0.01);

        let texts = painted_texts(&frame.full);
        assert!(
            texts.iter().any(|(t, _, _)| t == "Opus"),
            "stabilized popover must paint model rows in theme {theme_id:?}"
        );
        for (txt, rect, _) in texts {
            assert!(
                rect.right() <= harness.viewport.x + 1.0
                    && rect.bottom() <= harness.viewport.y + 1.0,
                "text `{txt}` overflowed viewport in theme {theme_id:?} (dark={dark}): {rect:?}"
            );
        }
    }

    // 3. Trigger chip toggle open -> toggle close (must NOT reopen on click release) & outside click.
    theme::apply(&ctx, true, None);
    picker.reset();
    let f_init = step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    let chip_pos = f_init.chip.rect.unwrap().center();

    step_header_and_popover(&ctx, &mut picker, &harness, pointer_down_events(chip_pos));
    step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(chip_pos));
    assert!(picker.is_open(), "1st chip click opens popover");

    step_header_and_popover(&ctx, &mut picker, &harness, pointer_down_events(chip_pos));
    let f_toggle_close =
        step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(chip_pos));
    assert!(!picker.is_open(), "2nd chip click closes popover cleanly");
    assert_eq!(
        f_toggle_close.picker.closed,
        Some(PickerCloseReason::TriggerToggle)
    );
    assert!(!PickerCloseReason::TriggerToggle.refocus_pane());

    // Open again, then click outside -> OutsideClick (`refocus_pane == false`).
    step_header_and_popover(&ctx, &mut picker, &harness, pointer_down_events(chip_pos));
    step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(chip_pos));
    step_header_and_popover(&ctx, &mut picker, &harness, vec![]);
    assert!(picker.is_open());
    let outside = Pos2::new(760.0, 560.0);
    let f_outside =
        step_header_and_popover(&ctx, &mut picker, &harness, pointer_down_events(outside));
    step_header_and_popover(&ctx, &mut picker, &harness, pointer_up_events(outside));
    assert!(!picker.is_open());
    assert_eq!(
        f_outside.picker.closed,
        Some(PickerCloseReason::OutsideClick)
    );
    assert!(!PickerCloseReason::OutsideClick.refocus_pane());

    // 4. Keyboard navigation (`ArrowDown`, `ArrowUp`, `End`, `Home`, `Enter`, `Space`)
    //    + 1-step `Escape` isolation against a parent host modal watching `Escape`.
    //    Claude rows: 0 = Agent's own default, 1 = Fable, 2 = Opus (checked!), 3 = Sonnet, 4 = Haiku.
    let saved_opus = json!({ "model": "opus" });
    let modal_ctx = Context::default();
    theme::apply(&modal_ctx, true, None);
    let mut env = ParentModalEnv {
        ctx: modal_ctx,
        picker: SessionOptionPickerUi::new(),
        defaults: &defaults,
        shown: &shown,
        session_options: Some(&saved_opus),
        viewport: harness.viewport,
        parent_modal_closed: false,
    };

    let (_, chip_id_opt) = step_parent_modal(&mut env, vec![]);
    let chip_id = chip_id_opt.unwrap();
    env.picker
        .open_at_point(&env.ctx, Pos2::new(24.0, 28.0), Some(chip_id));
    step_parent_modal(&mut env, vec![]);
    step_parent_modal(&mut env, vec![]);
    assert!(env.picker.is_open());

    // Initial active row matches checked `"Opus"` (row 2):
    // `ArrowDown` x2 -> `"Haiku"` (row 4), `ArrowUp` x1 -> `"Sonnet"` (row 3), `Enter` -> `"sonnet"`.
    step_parent_modal(&mut env, vec![key_event(Key::ArrowDown, true, false)]);
    step_parent_modal(&mut env, vec![key_event(Key::ArrowDown, true, false)]);
    step_parent_modal(&mut env, vec![key_event(Key::ArrowUp, true, false)]);
    let (enter_out, _) = step_parent_modal(&mut env, vec![key_event(Key::Enter, true, false)]);
    assert_eq!(
        enter_out.event,
        Some(SessionPickerEvent::Set {
            option_id: "model".into(),
            value: Some(json!("sonnet")),
        })
    );

    // `End` jumps to the last interactive item, `Home` jumps back to row 0 (`sop-model-default`),
    // and `Space` activates row 0 (`value: None`).
    step_parent_modal(&mut env, vec![key_event(Key::End, true, false)]);
    step_parent_modal(&mut env, vec![key_event(Key::Home, true, false)]);
    let (space_out, _) = step_parent_modal(&mut env, vec![key_event(Key::Space, true, false)]);
    assert_eq!(
        space_out.event,
        Some(SessionPickerEvent::Set {
            option_id: "model".into(),
            value: None,
        })
    );

    // Escape #1 (+ held repeat frame) closes ONLY the popover, restores focus to `chip_id`,
    // and does NOT close the parent modal until Escape is released and pressed a second time.
    let (esc1_out, _) = step_parent_modal(&mut env, vec![key_event(Key::Escape, true, false)]);
    assert!(!env.picker.is_open());
    assert!(esc1_out.escape_consumed);
    assert_eq!(esc1_out.closed, Some(PickerCloseReason::Escape));
    assert!(PickerCloseReason::Escape.refocus_pane());
    assert!(
        !env.parent_modal_closed,
        "Esc #1 must not close parent modal"
    );
    assert_eq!(env.ctx.memory(|m| m.focused()), Some(chip_id));

    step_parent_modal(&mut env, vec![key_event(Key::Escape, true, true)]);
    assert!(
        !env.parent_modal_closed,
        "Held Esc #1 repeat must not close parent modal"
    );
    assert_eq!(env.ctx.memory(|m| m.focused()), Some(chip_id));

    step_parent_modal(&mut env, vec![key_event(Key::Escape, false, false)]);
    step_parent_modal(&mut env, vec![key_event(Key::Escape, true, false)]);
    assert!(
        env.parent_modal_closed,
        "Esc #2 after release closes parent modal"
    );
}
