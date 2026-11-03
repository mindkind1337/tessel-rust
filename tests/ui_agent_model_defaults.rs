//! Standalone headless `egui` integration tests for `src/agent_model_defaults_ui.rs`
//! (`AgentModelDefaults.vue`, `ThemedSelect.vue`, `sessionOptionLabels.js`, `cursorModels.js`,
//! capability gating, `220px` flex-wrap layout, and 3-step `Escape` inside `settings_surface::show`).

#[allow(dead_code)]
#[path = "../src/agent_env.rs"]
mod agent_env;

#[allow(dead_code)]
#[path = "../src/agent_arg_options.rs"]
mod agent_arg_options;

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
#[path = "../src/settings_surface.rs"]
mod settings_surface;

#[allow(dead_code)]
#[path = "../src/agent_model_defaults_ui.rs"]
mod agent_model_defaults_ui;

use agent_model_defaults::Defaults;
use agent_model_defaults_ui::{
    AgentModelDefaultsUi, Capabilities, Input, ListedMeta, Output, ProbeError, ProbeState,
    agent_own_default_label, choice_value_label, decompose_cursor_model, defaults_section_title,
    is_launch_option, is_live_fast_option, model_choice_label, model_description,
    model_field_label, option_default_label, probe_error_text, refresh_models_hint,
    refresh_models_label, session_choice_label, session_option_label, session_pill_label,
    set_default_model, set_default_option, values_on_listed_row,
};
use eframe::egui::{
    self, Color32, Context, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape,
    Vec2,
};
use serde_json::{Value, json};

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

fn find_text_rect(output: &egui::FullOutput, target: &str) -> Option<Rect> {
    painted_texts(output)
        .into_iter()
        .find(|(text, _, _)| text == target)
        .map(|(_, rect, _)| rect)
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

struct HarnessParams<'a> {
    agent_id: &'a str,
    agent_name: &'a str,
    discovered: Option<&'a [Value]>,
    listed_meta: Option<ListedMeta<'a>>,
    probe: ProbeState<'a>,
    capabilities: Capabilities,
    french: bool,
    width: f32,
    enabled: bool,
}

impl Default for HarnessParams<'static> {
    fn default() -> Self {
        Self {
            agent_id: "codex",
            agent_name: "Codex CLI",
            discovered: None,
            listed_meta: None,
            probe: ProbeState::Idle,
            capabilities: Capabilities::default(),
            french: false,
            width: 620.0,
            enabled: true,
        }
    }
}

fn run_frame(
    ctx: &Context,
    ui_state: &mut AgentModelDefaultsUi,
    defaults: &mut Defaults,
    params: &HarnessParams<'_>,
    events: Vec<Event>,
) -> (egui::FullOutput, Output, f32) {
    let mut frame_out = Output::default();
    let mut right_edge = 0.0_f32;
    let raw = RawInput {
        screen_rect: Some(Rect::from_min_size(
            Pos2::ZERO,
            Vec2::new(params.width, 720.0),
        )),
        events,
        focused: true,
        ..Default::default()
    };
    let full = ctx.run(raw, |ctx| {
        if params.enabled {
            ui_state.handle_escape(ctx);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                ui.set_max_width(params.width);
                ui.add_enabled_ui(params.enabled, |ui| {
                    frame_out = ui_state.show(
                        ui,
                        Input {
                            agent_id: params.agent_id,
                            agent_name: params.agent_name,
                            defaults,
                            discovered_models: params.discovered,
                            listed_meta: params.listed_meta,
                            probe: params.probe.clone(),
                            capabilities: params.capabilities,
                            french: params.french,
                            show_section_header: true,
                        },
                    );
                    right_edge = ui.min_rect().right();
                });
            });
    });
    if let Some(updated) = &frame_out.changed {
        *defaults = updated.clone();
    }
    (full, frame_out, right_edge)
}

fn click_at(
    ctx: &Context,
    ui_state: &mut AgentModelDefaultsUi,
    defaults: &mut Defaults,
    params: &HarnessParams<'_>,
    pos: Pos2,
) -> (egui::FullOutput, Output) {
    run_frame(
        ctx,
        ui_state,
        defaults,
        params,
        vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    let (full, out, _) = run_frame(
        ctx,
        ui_state,
        defaults,
        params,
        vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    (full, out)
}

#[test]
fn en_and_fr_labels_descriptions_pills_and_probe_errors_match_vue_locales() {
    assert_eq!(defaults_section_title(false), "Default model");
    assert_eq!(defaults_section_title(true), "Modèle par défaut");
    assert_eq!(model_field_label(false), "Model");
    assert_eq!(model_field_label(true), "Modèle");
    assert_eq!(agent_own_default_label(false), "Agent's own default");
    assert_eq!(agent_own_default_label(true), "Par défaut de l'agent");
    assert_eq!(option_default_label(false), "Default");
    assert_eq!(option_default_label(true), "Par défaut");
    assert_eq!(refresh_models_label(false), "Refresh models");
    assert_eq!(refresh_models_label(true), "Actualiser les modèles");
    assert!(refresh_models_hint("Codex CLI", false).contains("no prompt is sent"));
    assert!(refresh_models_hint("Codex CLI", true).contains("aucune requête"));

    let reasoning = json!({"id": "effort", "label": "Reasoning effort"});
    let effort = json!({"id": "effort", "label": "Effort"});
    let fast = json!({"id": "fastMode", "label": "Fast mode"});
    let thinking = json!({"id": "thinking", "label": "Thinking"});
    assert_eq!(session_option_label(&reasoning, false), "Reasoning effort");
    assert_eq!(
        session_option_label(&reasoning, true),
        "Effort de raisonnement"
    );
    assert_eq!(session_option_label(&effort, false), "Effort");
    assert_eq!(session_option_label(&effort, true), "Effort");
    assert_eq!(session_option_label(&fast, false), "Fast mode");
    assert_eq!(session_option_label(&fast, true), "Mode rapide");
    assert_eq!(session_option_label(&thinking, false), "Thinking");
    assert_eq!(session_option_label(&thinking, true), "Réflexion");

    let expected_choices = [
        ("off", "Off", "Désactivé"),
        ("none", "None", "Aucune"),
        ("minimal", "Minimal", "Minimal"),
        ("low", "Low", "Basse"),
        ("medium", "Medium", "Moyenne"),
        ("high", "High", "Haute"),
        ("xhigh", "Extra high", "Extra élevé"),
        ("max", "Max", "Max"),
        ("ultra", "Ultra", "Ultra"),
    ];
    for (val, en, fr) in expected_choices {
        let choice = json!({"value": val, "label": en});
        assert_eq!(session_choice_label(&choice, false), en);
        assert_eq!(session_choice_label(&choice, true), fr);
        assert_eq!(choice_value_label(val, true), Some(fr));
    }

    let opus = json!({"id": "opus", "label": "Claude Opus", "description": "Best for everyday, complex tasks"});
    assert_eq!(
        model_description(&opus, false),
        "Best for everyday, complex tasks"
    );
    assert_eq!(
        model_description(&opus, true),
        "Idéal pour les tâches complexes du quotidien"
    );
    let grok = json!({"id": "grok-4.6", "label": "Grok 4.6", "description": "xAI's latest frontier model"});
    assert_eq!(
        model_description(&grok, true),
        "Le dernier modèle de pointe de xAI"
    );

    let models = vec![opus.clone(), grok];
    assert_eq!(model_choice_label(&models, "opus"), "Claude Opus");
    assert_eq!(model_choice_label(&models, "custom-1"), "custom-1");
    assert_eq!(session_pill_label(&models, None, false), "Model");
    assert_eq!(session_pill_label(&models, None, true), "Modèle");
    let pill_vals = json!({"model": "opus", "effort": "xhigh", "fastMode": true});
    assert_eq!(
        session_pill_label(&models, Some(&pill_vals), false),
        "Claude Opus · Extra high · Fast"
    );
    assert_eq!(
        session_pill_label(&models, Some(&pill_vals), true),
        "Claude Opus · Extra élevé · Rapide"
    );

    assert_eq!(
        probe_error_text(&ProbeError::NotFound, "Codex", true),
        "Codex est introuvable sur cet ordinateur."
    );
    assert_eq!(
        probe_error_text(&ProbeError::Timeout, "Codex", false),
        "Codex did not list its models within 60 s."
    );
    assert_eq!(
        probe_error_text(&ProbeError::TooMuch, "Codex", true),
        "Codex a renvoyé trop de données de modèles."
    );
    assert_eq!(
        probe_error_text(&ProbeError::Empty, "Codex", true),
        "Codex n'a listé aucun modèle ; la liste intégrée est conservée."
    );
    assert_eq!(
        probe_error_text(&ProbeError::Unsupported, "Gemini", true),
        "Gemini ne peut pas lister ses modèles."
    );
    assert_eq!(
        probe_error_text(
            &ProbeError::Failed {
                detail: Some("exit 1")
            },
            "Codex",
            false
        ),
        "Codex could not list its models: exit 1"
    );
    assert_eq!(
        probe_error_text(&ProbeError::Failed { detail: None }, "Codex", true),
        "Codex n'a pas pu lister ses modèles."
    );
}

#[test]
fn cursor_boolean_switches_never_claim_thinking_on_when_unset_and_match_launch_flags() {
    let ctx = Context::default();
    theme::apply(&ctx, true, None);
    let mut ui_state = AgentModelDefaultsUi::new();
    let mut defaults = Defaults {
        model: Some("claude-opus-4-8".into()),
        ..Default::default()
    };
    let params = HarnessParams {
        agent_id: "cursor",
        agent_name: "Cursor Agent",
        french: true,
        ..Default::default()
    };

    let (full, out, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &params, vec![]);
    assert!(out.supported);
    assert!(out.changed.is_none());

    // French labels must all be rendered, and no 3-way dropdown "Activé"/"On" may exist.
    let texts: Vec<String> = painted_texts(&full)
        .into_iter()
        .map(|(t, _, _)| t)
        .collect();
    assert!(texts.iter().any(|t| t == "Modèle par défaut"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Modèle"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Claude Opus 4.8"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Réflexion"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Effort"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Par défaut"), "{texts:?}");
    assert!(
        !texts.iter().any(|t| t == "Activé" || t == "On"),
        "{texts:?}"
    );

    // Untouched `thinking` must launch WITHOUT `-thinking`, and clicking the switch once
    // must turn `thinking` ON (`true`), matching actual launch flags.
    assert_eq!(
        agent_model_launch::text("cursor", &defaults.values().unwrap(), "", None),
        "--model claude-opus-4-8"
    );

    let reflexion_rect = find_text_rect(&full, "Réflexion").expect("Réflexion switch label");
    let (_, toggle_on) = click_at(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        reflexion_rect.center(),
    );
    assert!(
        toggle_on.changed.is_some(),
        "first click on unchecked Thinking switch must turn it ON"
    );
    assert_eq!(defaults.values().unwrap()["thinking"], Value::Bool(true));
    assert_eq!(
        agent_model_launch::text("cursor", &defaults.values().unwrap(), "", None),
        "--model claude-opus-4-8-thinking"
    );

    // Space key on the focused switch toggles it back OFF (`false`).
    let (_, toggle_off, _) = run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::Space, true, false)],
    );
    assert!(toggle_off.changed.is_some());
    assert_eq!(defaults.values().unwrap()["thinking"], Value::Bool(false));
    assert_eq!(
        agent_model_launch::text("cursor", &defaults.values().unwrap(), "", None),
        "--model claude-opus-4-8"
    );
}

#[test]
fn cursor_saved_variant_id_decomposes_onto_grouped_row_and_promotes_siblings_on_edit() {
    let grouped = vec![
        json!({
            "id": "auto",
            "label": "Auto",
            "isDefault": true,
            "options": []
        }),
        json!({
            "id": "gpt-5.3-codex",
            "label": "GPT-5.3 Codex",
            "variants": [
                {"id": "gpt-5.3-codex-low", "effort": "low", "fast": false, "thinking": false},
                {"id": "gpt-5.3-codex", "effort": "medium", "fast": false, "thinking": false},
                {"id": "gpt-5.3-codex-high", "effort": "high", "fast": false, "thinking": false},
                {"id": "gpt-5.3-codex-high-fast", "effort": "high", "fast": true, "thinking": false},
                {"id": "gpt-5.3-codex-xhigh", "effort": "xhigh", "fast": false, "thinking": false},
                {"id": "gpt-5.3-codex-xhigh-fast", "effort": "xhigh", "fast": true, "thinking": false}
            ],
            "efforts": ["low", "medium", "high", "xhigh"],
            "defaultEffort": "medium",
            "hasFast": true,
            "hasThinking": false,
            "defaultThinking": false,
            "options": [
                {
                    "id": "effort",
                    "label": "Effort",
                    "category": "thought_level",
                    "kind": {
                        "type": "select",
                        "choices": [
                            {"value": "low", "label": "Low"},
                            {"value": "medium", "label": "Medium"},
                            {"value": "high", "label": "High"},
                            {"value": "xhigh", "label": "Extra high"}
                        ],
                        "defaultValue": "medium"
                    },
                    "apply": {"composedIntoModel": true}
                },
                {
                    "id": "fastMode",
                    "label": "Fast mode",
                    "category": "mode",
                    "kind": {"type": "boolean", "defaultValue": false},
                    "apply": {"composedIntoModel": true}
                }
            ]
        }),
    ];

    assert_eq!(
        decompose_cursor_model(&grouped, "gpt-5.3-codex-high-fast").map(Value::Object),
        Some(json!({"model": "gpt-5.3-codex", "effort": "high", "fastMode": true}))
    );

    let ctx = Context::default();
    theme::apply(&ctx, true, None);
    let mut ui_state = AgentModelDefaultsUi::new();
    let mut defaults = Defaults {
        model: Some("gpt-5.3-codex-high-fast".into()),
        ..Default::default()
    };
    let params = HarnessParams {
        agent_id: "cursor",
        agent_name: "Cursor Agent",
        discovered: Some(&grouped),
        ..Default::default()
    };

    let (full, _, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &params, vec![]);
    let texts: Vec<String> = painted_texts(&full)
        .into_iter()
        .map(|(t, _, _)| t)
        .collect();
    assert!(texts.iter().any(|t| t == "GPT-5.3 Codex"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "High"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Fast mode"), "{texts:?}");
    assert!(
        !texts.iter().any(|t| t == "gpt-5.3-codex-high-fast"),
        "decomposed variant must not appear as a raw fallback label: {texts:?}"
    );

    // Open Effort dropdown ("High") and pick "Extra high": must promote `defaults.model`
    // to `"gpt-5.3-codex"` while preserving sibling `fastMode: true`.
    let effort_pos = find_text_rect(&full, "High").unwrap().center();
    click_at(&ctx, &mut ui_state, &mut defaults, &params, effort_pos);
    // A newly created egui Area first performs an invisible sizing pass.
    let (popup_frame, _, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &params, vec![]);
    assert!(ui_state.is_popup_open());
    let xhigh_pos = find_text_rect(&popup_frame, "Extra high").unwrap().center();
    let (_, edit_out) = click_at(&ctx, &mut ui_state, &mut defaults, &params, xhigh_pos);
    assert!(edit_out.changed.is_some());
    assert!(!ui_state.is_popup_open());
    assert_eq!(defaults.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(
        defaults.values().unwrap(),
        json!({"model": "gpt-5.3-codex", "effort": "xhigh", "fastMode": true})
    );
    assert_eq!(
        agent_model_launch::text("cursor", &defaults.values().unwrap(), "", Some(&grouped)),
        "--model gpt-5.3-codex-xhigh-fast"
    );

    // Also verify helper functions directly (`values_on_listed_row` & `set_default_option`).
    let mut direct = Defaults {
        model: Some("gpt-5.3-codex-high-fast".into()),
        ..Default::default()
    };
    let shown = values_on_listed_row(&grouped, direct.values().as_ref()).unwrap();
    assert_eq!(shown["effort"], "high");
    set_default_option(&grouped, &mut direct, "effort", None);
    assert_eq!(
        direct.values().unwrap(),
        json!({"model": "gpt-5.3-codex", "fastMode": true})
    );
}

#[test]
fn capability_gates_hide_unwired_restart_and_live_fast_notes_and_enable_cleanly() {
    let claude_cat = agent_model_catalog::find("claude").unwrap();
    let opus_opts = agent_model_catalog::model_options(claude_cat, None, "opus");
    assert!(opus_opts.iter().any(|o| is_launch_option("claude", o)));
    assert!(opus_opts.iter().any(|o| is_live_fast_option("claude", o)));

    let ctx = Context::default();
    theme::apply(&ctx, true, None);
    let mut ui_state = AgentModelDefaultsUi::new();
    let mut defaults = Defaults {
        model: Some("opus".into()),
        ..Default::default()
    };

    // 1. Default capabilities (all false): no Refresh button, no Restart to apply, no pane menu > Model.
    let base_params = HarnessParams {
        agent_id: "claude",
        agent_name: "Claude Code",
        capabilities: Capabilities::default(),
        french: false,
        ..Default::default()
    };
    let (full_default, _, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &base_params, vec![]);
    let all_default_text = painted_texts(&full_default)
        .into_iter()
        .map(|(t, _, _)| t)
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        all_default_text.contains(
            "Built-in model list. The agent may also offer other models in its own settings."
        ),
        "{all_default_text}"
    );
    assert!(
        all_default_text.contains("Applies to panes you start from now on."),
        "{all_default_text}"
    );
    assert!(
        !all_default_text.contains("Restart to apply"),
        "{all_default_text}"
    );
    assert!(
        !all_default_text.contains("pane menu > Model"),
        "{all_default_text}"
    );
    assert!(
        !all_default_text.contains("Refresh models"),
        "{all_default_text}"
    );

    // Empty-seed catalog (`opencode`) without probe wired never claims a built-in list exists.
    let mut empty_defaults = Defaults::default();
    let opencode_params = HarnessParams {
        agent_id: "opencode",
        agent_name: "OpenCode",
        capabilities: Capabilities::default(),
        french: true,
        ..Default::default()
    };
    let (full_opencode, _, _) = run_frame(
        &ctx,
        &mut ui_state,
        &mut empty_defaults,
        &opencode_params,
        vec![],
    );
    let opencode_text = painted_texts(&full_opencode)
        .into_iter()
        .map(|(t, _, _)| t)
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        opencode_text.contains("Aucune liste de modèles intégrée pour OpenCode."),
        "{opencode_text}"
    );

    // 2. Full capabilities wired: Refresh button + live `/fast` note + `Restart to apply` note + probe error warn color.
    let full_caps = Capabilities {
        can_refresh_models: true,
        running_pane_model_menu: true,
        running_pane_restart_badge: true,
    };
    let wired_params = HarnessParams {
        agent_id: "claude",
        agent_name: "Claude Code",
        capabilities: full_caps,
        french: true,
        ..Default::default()
    };
    let (full_wired, _, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &wired_params, vec![]);
    let wired_text = painted_texts(&full_wired)
        .into_iter()
        .map(|(t, _, _)| t)
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        wired_text.contains(
            "Liste intégrée. Actualiser demande à Claude Code les modèles de votre compte."
        ),
        "{wired_text}"
    );
    assert!(
        wired_text.contains("Le mode rapide se change dans un panneau en cours d'exécution (menu du panneau > Modèle)."),
        "{wired_text}"
    );
    assert!(
        wired_text.contains("S'applique aux panneaux que vous démarrez désormais ; ceux en cours affichent Redémarrer pour appliquer."),
        "{wired_text}"
    );

    let refresh_pos = find_text_rect(&full_wired, "Actualiser les modèles")
        .expect("Refresh button visible when can_refresh_models is true")
        .center();
    let (_, refresh_out) = click_at(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &wired_params,
        refresh_pos,
    );
    assert!(refresh_out.refresh_models);

    // While busy, Refresh button does not fire again.
    let busy_params = HarnessParams {
        probe: ProbeState::Busy,
        ..wired_params
    };
    let (full_busy, _, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &busy_params, vec![]);
    let busy_pos = find_text_rect(&full_busy, "Actualiser les modèles")
        .unwrap()
        .center();
    let (_, busy_click) = click_at(&ctx, &mut ui_state, &mut defaults, &busy_params, busy_pos);
    assert!(!busy_click.refresh_models);

    // Listed metadata & error warn color.
    let listed_params = HarnessParams {
        listed_meta: Some(ListedMeta {
            count: 4,
            date_label: "3 nov. 2026, 17:50",
        }),
        probe: ProbeState::Idle,
        ..busy_params
    };
    let (full_listed, _, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &listed_params, vec![]);
    assert!(
        painted_texts(&full_listed)
            .iter()
            .any(|(t, _, _)| t.contains("4 modèles listés par Claude Code le 3 nov. 2026, 17:50."))
    );

    let err_params = HarnessParams {
        listed_meta: None,
        probe: ProbeState::Error(ProbeError::Timeout),
        ..listed_params
    };
    let (full_err, _, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &err_params, vec![]);
    let warn_color = ctx.style().visuals.warn_fg_color;
    let status_entry = painted_texts(&full_err)
        .into_iter()
        .find(|(t, _, _)| t.contains("Claude Code n'a pas listé ses modèles en 60 s."))
        .expect("timeout probe error status");
    assert_eq!(status_entry.2, warn_color);
}

#[test]
fn layout_places_220px_fields_side_by_side_on_desktop_and_wraps_cleanly_at_300px() {
    let ctx = Context::default();
    let mut ui_state = AgentModelDefaultsUi::new();
    let mut defaults = Defaults {
        model: Some("gpt-5.6-sol".into()),
        ..Default::default()
    };

    for theme_id in theme_palettes::THEMES {
        theme::apply(&ctx, true, Some(theme_id));
        let wide = HarnessParams {
            width: 620.0,
            ..Default::default()
        };
        let (full, _, right) = run_frame(&ctx, &mut ui_state, &mut defaults, &wide, vec![]);
        assert!(
            right <= 620.01,
            "wide right edge overflow on {theme_id:?}: {right}"
        );
        let model_label = find_text_rect(&full, "Model").expect("Model field hint");
        let effort_label = find_text_rect(&full, "Reasoning effort").expect("Effort field hint");
        assert!(
            (model_label.top() - effort_label.top()).abs() < 1.0,
            "Model and Effort must sit side-by-side on 620px desktop ({model_label:?} vs {effort_label:?})"
        );
        assert!(
            effort_label.left() >= model_label.left() + 230.0,
            "220px Model + 12px gap before Effort ({model_label:?} vs {effort_label:?})"
        );
    }

    // Legacy light mode + narrow 300px viewport: Effort wraps underneath Model, zero horizontal overflow.
    theme::apply(&ctx, false, None);
    let narrow = HarnessParams {
        width: 300.0,
        french: true,
        ..Default::default()
    };
    let (full_narrow, _, narrow_right) =
        run_frame(&ctx, &mut ui_state, &mut defaults, &narrow, vec![]);
    assert!(narrow_right <= 300.01, "narrow overflow: {narrow_right}");
    let model_fr = find_text_rect(&full_narrow, "Modèle").unwrap();
    let effort_fr = find_text_rect(&full_narrow, "Effort de raisonnement").unwrap();
    assert!(
        effort_fr.top() >= model_fr.bottom() + 28.0,
        "Effort must wrap below Model at 300px ({model_fr:?} vs {effort_fr:?})"
    );
    for (text, rect, _) in painted_texts(&full_narrow) {
        assert!(
            rect.right() <= 300.5,
            "text {text:?} overflowed 300px viewport: {rect:?}"
        );
    }
}

#[test]
fn keyboard_navigation_typeahead_tab_outside_click_and_clearing_preserve_per_model_values() {
    let ctx = Context::default();
    theme::apply(&ctx, true, None);
    let mut ui_state = AgentModelDefaultsUi::new();
    let mut defaults = Defaults::default();
    let params = HarnessParams::default();

    let (_, init_out, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &params, vec![]);
    let model_trigger_id = init_out.focus_ids[0];
    ctx.memory_mut(|m| m.request_focus(model_trigger_id));
    run_frame(&ctx, &mut ui_state, &mut defaults, &params, vec![]);

    // Type-ahead 'g' opens popup and jumps directly to the first "GPT-..." model; Enter selects it.
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![Event::Text("g".into())],
    );
    assert!(ui_state.is_popup_open());
    let (_, pick_out, _) = run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::Enter, true, false)],
    );
    assert!(!ui_state.is_popup_open());
    let chosen_first_gpt = pick_out
        .changed
        .as_ref()
        .and_then(|d| d.model.clone())
        .expect("type-ahead + Enter must pick a GPT model");
    assert!(chosen_first_gpt.starts_with("gpt-"), "{chosen_first_gpt}");
    assert!(ctx.memory(|m| m.has_focus(model_trigger_id)));

    // Focus Effort select, open with End (jumps to last choice), step up once with ArrowUp, pick with Space.
    let (_, with_effort_out, _) = run_frame(&ctx, &mut ui_state, &mut defaults, &params, vec![]);
    let effort_trigger_id = with_effort_out.focus_ids[1];
    ctx.memory_mut(|m| m.request_focus(effort_trigger_id));
    run_frame(&ctx, &mut ui_state, &mut defaults, &params, vec![]);
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::End, true, false)],
    );
    assert!(ui_state.is_popup_open());
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::ArrowUp, true, false)],
    );
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::Space, true, false), Event::Text(" ".into())],
    );
    assert!(!ui_state.is_popup_open());
    let saved_effort = defaults.values().unwrap()["effort"]
        .as_str()
        .unwrap()
        .to_owned();

    // Open Effort popup and verify Tab closes it cleanly; open again and verify outside click closes it.
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::ArrowDown, true, false)],
    );
    assert!(ui_state.is_popup_open());
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::Tab, true, false)],
    );
    assert!(!ui_state.is_popup_open());

    ctx.memory_mut(|m| m.request_focus(model_trigger_id));
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::Home, true, false)],
    );
    assert!(ui_state.is_popup_open());
    // Outside click at bottom-right closes popup without changing selection.
    click_at(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        Pos2::new(600.0, 700.0),
    );
    assert!(!ui_state.is_popup_open());
    assert_eq!(defaults.model.as_deref(), Some(chosen_first_gpt.as_str()));

    // Clear model back to "Agent's own default" via Home + Enter -> `model` becomes `None`,
    // while `values_by_model` retains `saved_effort` for a future re-selection!
    ctx.memory_mut(|m| m.request_focus(model_trigger_id));
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::Home, true, false)],
    );
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::Enter, true, false)],
    );
    assert!(defaults.model.is_none());
    assert!(defaults.values().is_none());
    set_default_model(&mut defaults, Some(chosen_first_gpt));
    assert_eq!(defaults.values().unwrap()["effort"], saved_effort);

    // Disabling the UI while a popup is open closes the popup immediately.
    ctx.memory_mut(|m| m.request_focus(model_trigger_id));
    run_frame(
        &ctx,
        &mut ui_state,
        &mut defaults,
        &params,
        vec![key_event(Key::ArrowDown, true, false)],
    );
    assert!(ui_state.is_popup_open());
    let disabled_params = HarnessParams {
        enabled: false,
        ..params
    };
    run_frame(&ctx, &mut ui_state, &mut defaults, &disabled_params, vec![]);
    assert!(!ui_state.is_popup_open());
}

#[test]
fn three_step_escape_inside_settings_modal_closes_popup_then_folds_details_then_closes_settings() {
    struct ModalEnv {
        ctx: Context,
        ui_state: AgentModelDefaultsUi,
        defaults: Defaults,
        details_open: bool,
        detail_escape_held: bool,
        detail_focus: Vec<Id>,
        row_id: Id,
        external_search_id: Id,
    }

    fn step(env: &mut ModalEnv, events: Vec<Event>) -> (egui::FullOutput, bool) {
        let mut settings_closed = false;
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            events,
            focused: true,
            ..Default::default()
        };
        let full = env.ctx.clone().run(raw, |ctx| {
            // Mirrors `AgentsPanel::handle_escape` + `app.rs:2921-3004`.
            let down = ctx.input(|i| i.key_down(Key::Escape));
            if !down {
                env.detail_escape_held = false;
            }
            let mut agents_escape_consumed = env.ui_state.handle_escape(ctx);
            if !agents_escape_consumed {
                let in_detail = ctx.memory(|m| match m.focused() {
                    Some(id) => env.detail_focus.contains(&id),
                    None if down => env
                        .detail_focus
                        .iter()
                        .any(|id| m.had_focus_last_frame(*id)),
                    None => false,
                });
                if (env.detail_escape_held || (env.details_open && in_detail))
                    && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                {
                    env.details_open = false;
                    ctx.memory_mut(|m| m.request_focus(env.row_id));
                    env.detail_escape_held = down;
                    agents_escape_consumed = true;
                }
            }

            let modal = settings_surface::show(ctx, |ui, nav_w| {
                ui.horizontal_top(|ui| {
                    let search_resp = ui.interact(
                        Rect::from_min_size(ui.min_rect().min, Vec2::new(nav_w - 20.0, 28.0)),
                        env.external_search_id,
                        egui::Sense::click(),
                    );
                    if search_resp.has_focus() {
                        ui.memory_mut(|m| {
                            m.set_focus_lock_filter(
                                env.external_search_id,
                                egui::EventFilter {
                                    escape: true,
                                    ..Default::default()
                                },
                            );
                        });
                    }
                    settings_surface::body(ui, |ui| {
                        let row_resp = ui.interact(
                            Rect::from_min_size(ui.cursor().min, Vec2::new(300.0, 32.0)),
                            env.row_id,
                            egui::Sense::click(),
                        );
                        let _ = row_resp;
                        ui.advance_cursor_after_rect(Rect::from_min_size(
                            ui.cursor().min,
                            Vec2::new(300.0, 36.0),
                        ));
                        env.detail_focus.clear();
                        if env.details_open {
                            env.detail_focus.push(env.row_id);
                            let out = env.ui_state.show(
                                ui,
                                Input {
                                    agent_id: "codex",
                                    agent_name: "Codex CLI",
                                    defaults: &env.defaults,
                                    discovered_models: None,
                                    listed_meta: None,
                                    probe: ProbeState::Idle,
                                    capabilities: Capabilities::default(),
                                    french: false,
                                    show_section_header: true,
                                },
                            );
                            env.detail_focus.extend(out.focus_ids);
                        }
                    });
                });
            });
            if !agents_escape_consumed && modal.should_close() {
                settings_closed = true;
            }
        });
        (full, settings_closed)
    }

    let ctx = Context::default();
    theme::apply(&ctx, true, None);
    let mut env = ModalEnv {
        ctx,
        ui_state: AgentModelDefaultsUi::new(),
        defaults: Defaults {
            model: Some("gpt-5.6-sol".into()),
            ..Default::default()
        },
        details_open: true,
        detail_escape_held: false,
        detail_focus: Vec::new(),
        row_id: Id::new("agent-row-codex"),
        external_search_id: Id::new("settings-search-input"),
    };

    // Initial render + open the Model listbox by clicking "GPT-5.6 Sol".
    step(&mut env, vec![]);
    let (initial, closed0) = step(&mut env, vec![]);
    assert!(!closed0);
    let model_pos = find_text_rect(&initial, "GPT-5.6 Sol").unwrap().center();
    step(
        &mut env,
        vec![
            Event::PointerMoved(model_pos),
            Event::PointerButton {
                pos: model_pos,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ],
    );
    step(
        &mut env,
        vec![Event::PointerButton {
            pos: model_pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(env.ui_state.is_popup_open(), "Model listbox must be open");
    let model_trigger_id = env.detail_focus[1];

    // Prove the popup remains open across neutral frames, pointer hover moves over options,
    // scroll inside the popup, and multi-frame type-ahead before Escape is ever pressed.
    let (open_frame_a, closed_idle_a) = step(&mut env, vec![]);
    assert!(
        env.ui_state.is_popup_open(),
        "popup must survive neutral frame 1"
    );
    assert!(!closed_idle_a);
    let (_, closed_idle_b) = step(&mut env, vec![]);
    assert!(
        env.ui_state.is_popup_open(),
        "popup must survive neutral frame 2"
    );
    assert!(!closed_idle_b);

    let option_hover_pos = find_text_rect(&open_frame_a, "Agent's own default")
        .expect("popup option visible on neutral frame")
        .center();
    let (_, closed_hover) = step(&mut env, vec![Event::PointerMoved(option_hover_pos)]);
    assert!(
        env.ui_state.is_popup_open(),
        "popup must survive pointer hover move"
    );
    assert!(!closed_hover);

    let (_, closed_inner_scroll) = step(
        &mut env,
        vec![
            Event::PointerMoved(option_hover_pos),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, -24.0),
                modifiers: Modifiers::NONE,
            },
        ],
    );
    assert!(
        env.ui_state.is_popup_open(),
        "popup must survive scrolling inside popup"
    );
    assert!(!closed_inner_scroll);

    let (_, closed_typeahead) = step(&mut env, vec![Event::Text("g".into())]);
    assert!(
        env.ui_state.is_popup_open(),
        "popup must stay open during type-ahead"
    );
    assert!(!closed_typeahead);
    let (_, closed_arrow) = step(&mut env, vec![key_event(Key::ArrowDown, true, false)]);
    assert!(
        env.ui_state.is_popup_open(),
        "popup must stay open during ArrowDown"
    );
    assert!(!closed_arrow);

    // Esc #1 (+ held repeat frame): closes ONLY the listbox; details stay open, Settings stays open,
    // and focus stays on `model_trigger_id`.
    let (_, closed_esc1) = step(&mut env, vec![key_event(Key::Escape, true, false)]);
    assert!(
        !env.ui_state.is_popup_open(),
        "Esc #1 must close select popup"
    );
    assert!(env.details_open, "Esc #1 must NOT fold agent details");
    assert!(!closed_esc1, "Esc #1 must NOT close Settings modal");
    assert_eq!(env.ctx.memory(|m| m.focused()), Some(model_trigger_id));

    let (_, closed_esc1_repeat) = step(&mut env, vec![key_event(Key::Escape, true, true)]);
    assert!(
        env.details_open,
        "held Esc #1 repeat must NOT fold agent details"
    );
    assert!(
        !closed_esc1_repeat,
        "held Esc #1 repeat must NOT close Settings modal"
    );

    // Release Escape.
    step(&mut env, vec![key_event(Key::Escape, false, false)]);
    assert_eq!(env.ctx.memory(|m| m.focused()), Some(model_trigger_id));

    // Verify external focus isolation while details are open and popup is closed:
    // focusing Settings search does NOT get intercepted by `ui_state` or detail folding.
    env.ctx
        .memory_mut(|m| m.request_focus(env.external_search_id));
    step(&mut env, vec![]);
    // Move focus back to `model_trigger_id` to test Esc #2 -> Esc #3.
    env.ctx.memory_mut(|m| m.request_focus(model_trigger_id));
    step(&mut env, vec![]);

    // Esc #2 (+ held repeat frame): folds agent details, focuses `row_id`, keeps Settings modal open!
    let (_, closed_esc2) = step(&mut env, vec![key_event(Key::Escape, true, false)]);
    assert!(!env.details_open, "Esc #2 must fold agent details");
    assert!(!closed_esc2, "Esc #2 must NOT close Settings modal");
    assert_eq!(env.ctx.memory(|m| m.focused()), Some(env.row_id));

    let (_, closed_esc2_repeat) = step(&mut env, vec![key_event(Key::Escape, true, true)]);
    assert!(
        !closed_esc2_repeat,
        "held Esc #2 repeat must NOT close Settings modal"
    );

    // Release Escape, then Esc #3 closes the Settings modal!
    step(&mut env, vec![key_event(Key::Escape, false, false)]);
    let (_, closed_esc3) = step(&mut env, vec![key_event(Key::Escape, true, false)]);
    assert!(closed_esc3, "Esc #3 must close Settings modal");
}
