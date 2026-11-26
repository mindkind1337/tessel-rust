use super::{FocusInputTarget, TesselGpuiRoot, theme::GpuiThemeTokens};
use crate::{
    app::gpui_bridge::GpuiAction,
    settings_model::{SettingsPageId, SettingsSearchRow},
};
use gpui::{
    AnyElement, AppContext, Context, Entity, EntityInputHandler, FocusHandle, Focusable,
    KeyDownEvent, Subscription, Window, div, prelude::*, px,
};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, text_input};

pub(super) const ID: &str = "settings-auto-save-delay";
fn labels(french: bool) -> (&'static str, &'static str) {
    if french {
        (
            "Enregistrer après",
            "Millisecondes sans saisie (250 à 10000)",
        )
    } else {
        ("Save after", "Milliseconds without typing (250 to 10000)")
    }
}
pub(super) fn search_row(french: bool) -> SettingsSearchRow {
    let (label, hint) = labels(french);
    SettingsSearchRow {
        page: SettingsPageId::Editor,
        group_anchor: None,
        group_title: String::new(),
        row_id: ID.into(),
        text: format!("{label} {hint}"),
    }
}

struct Delay {
    base: u32,
    state: Entity<EditableTextState>,
    _focus: Subscription,
    _blur: Subscription,
}
pub(super) struct Controls {
    pub(super) swatches: [FocusHandle; 8],
    delay: Option<Delay>,
}
impl Controls {
    pub(super) fn new(cx: &mut Context<TesselGpuiRoot>) -> Self {
        Self {
            swatches: std::array::from_fn(|_| cx.focus_handle()),
            delay: None,
        }
    }
    pub(super) fn sync(
        &mut self,
        value: Option<u32>,
        w: &mut Window,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        let Some(base) = value else {
            self.delay = None;
            return;
        };
        if self
            .delay
            .as_ref()
            .is_some_and(|binding| binding.base == base)
        {
            return;
        }
        let state =
            cx.new(|cx| EditableTextState::new(StringStorage::from(base.to_string().as_str()), cx));
        let focus = state.read(cx).focus_handle(cx);
        let expected = state.clone();
        let focused = cx.on_focus(&focus, w, |root, _, _| {
            root.focus_input = FocusInputTarget::None
        });
        let blur = cx.on_blur(&focus, w, move |root, w, cx| {
            finish(root, &expected, 0, w, cx)
        });
        self.delay = Some(Delay {
            base,
            state,
            _focus: focused,
            _blur: blur,
        });
    }
    pub(super) fn render(
        &self,
        french: bool,
        t: &GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> AnyElement {
        let Some(binding) = &self.delay else {
            return div().into_any_element();
        };
        let (label, hint) = labels(french);
        let expected = binding.state.clone();
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .min_h(px(42.))
            .pl(px(16.))
            .border_b_1()
            .border_color(t.border)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .text_size(px(12.5))
                    .child(label)
                    .child(div().text_color(t.text_dim).text_size(px(11.)).child(hint)),
            )
            .child(
                text_input(ID)
                    .debug_selector(|| ID.into())
                    .state(binding.state.downgrade())
                    .aria_label(label)
                    .key_context("EditableText TesselAutoSaveDelay")
                    .capture_any_mouse_down(cx.listener(|root, e: &gpui::MouseDownEvent, _, _| {
                        if e.button == gpui::MouseButton::Left {
                            root.focus_input = FocusInputTarget::None;
                        }
                    }))
                    .capture_key_down(cx.listener(move |root, e: &KeyDownEvent, w, cx| {
                        let step = match e.keystroke.key.as_str() {
                            "up" => 1,
                            "down" => -1,
                            "enter" => 0,
                            _ => return,
                        };
                        if e.keystroke.modifiers.modified() {
                            return;
                        }
                        cx.stop_propagation();
                        finish(root, &expected, step, w, cx);
                    }))
                    .w(px(96.))
                    .px(px(8.))
                    .py(px(4.))
                    .border_1()
                    .border_color(t.border)
                    .rounded(px(4.))
                    .bg(t.term)
                    .text_color(t.text),
            )
            .into_any_element()
    }
}
fn finish(
    root: &mut TesselGpuiRoot,
    expected: &Entity<EditableTextState>,
    step: i32,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let Some(binding) = root
        .settings_controls
        .delay
        .as_mut()
        .filter(|d| &d.state == expected)
    else {
        return;
    };
    let base = binding.base;
    let value = expected.update(cx, |s, cx| {
        if s.marked_text_range(w, cx).is_some() {
            return None;
        }
        let raw = s
            .as_str()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(|v| v.trunc() as i32)
            .filter(|v| *v != 0)
            .unwrap_or(1000);
        let val = match step {
            1 => (raw.clamp(0, 10000) / 250 + 1) * 250,
            -1 => ((raw.clamp(1, 10250) - 1) / 250) * 250,
            _ => raw,
        }
        .clamp(250, 10000) as u32;
        let canonical = val.to_string();
        if canonical != s.as_str() {
            let len = s.as_str().encode_utf16().count();
            s.replace_text_in_range(Some(0..len), &canonical, w, cx);
        }
        Some(val)
    });
    if let Some(delay) = value
        && delay != base
    {
        binding.base = delay;
        root.dispatch_action(GpuiAction::SetAutoSaveDelay { base, delay }, cx);
    }
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    #[gpui::test]
    fn auto_save_delay_search_field_commits_on_enter_arrows_and_blur(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::SetLanguage { french: false });
            root.app
                .apply_gpui_action(GpuiAction::OpenSettingsPage("general".into()));
            root.app
                .apply_gpui_action(GpuiAction::SetSettingsSearch("milliseconds typing".into()));
            root
        });
        view.update(cx, |_, w, _| w.activate()).unwrap();
        cx.refresh().unwrap();
        assert!(
            gpui::VisualTestContext::from_window(view.into(), cx)
                .debug_bounds(ID)
                .is_none()
        );
        view.update(cx, |root, _, cx| {
            let mut prefs = root
                .app
                .gpui_snapshot(960., 640., 4.)
                .settings
                .unwrap()
                .editor_prefs;
            prefs.auto_save = true;
            root.dispatch_action(GpuiAction::UpdateEditorPrefs(prefs), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let point = visual.debug_bounds(ID).unwrap().center();
        visual.simulate_click(point, Default::default());
        let set = |cx: &mut gpui::TestAppContext, raw: &str| {
            view.update(cx, |root, w, cx| {
                let d = root.settings_controls.delay.as_ref().unwrap();
                assert!(d.state.read(cx).focus_handle(cx).is_focused(w));
                d.state.update(cx, |s, cx| {
                    let len = s.as_str().encode_utf16().count();
                    s.replace_text_in_range(Some(0..len), raw, w, cx);
                })
            })
            .unwrap();
        };
        let check = |cx: &mut gpui::TestAppContext, value: u32| {
            cx.refresh().unwrap();
            view.update(cx, |root, _, cx| {
                let d = root.settings_controls.delay.as_ref().unwrap();
                assert_eq!(d.state.read(cx).as_str(), value.to_string());
                assert_eq!(
                    root.app
                        .gpui_snapshot(960., 640., 4.)
                        .settings
                        .unwrap()
                        .editor_prefs
                        .auto_save_delay_ms,
                    value
                );
            })
            .unwrap();
        };
        set(cx, "875");
        cx.simulate_keystrokes(view.into(), "enter");
        check(cx, 875);
        cx.simulate_keystrokes(view.into(), "up");
        check(cx, 1000);
        cx.simulate_keystrokes(view.into(), "down");
        check(cx, 750);
        set(cx, "10");
        cx.simulate_keystrokes(view.into(), "enter");
        check(cx, 250);
        set(cx, "20000");
        cx.simulate_keystrokes(view.into(), "ctrl-f");
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert!(
                root.modal_query_input.focused(w, cx).is_some(),
                "Ctrl+F must move focus to search"
            );
        })
        .unwrap();
        check(cx, 10000);
        let saved: crate::workspace::Workspace =
            serde_json::from_slice(&std::fs::read(dir.path().join("workspace.json")).unwrap())
                .unwrap();
        assert_eq!(saved.editor_prefs.auto_save_delay_ms, 10000);
    }
}
