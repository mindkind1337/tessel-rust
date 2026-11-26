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

const DELAY_ID: &str = "settings-auto-save-delay";
#[derive(Clone, Copy)]
pub(super) enum Field {
    Delay,
    Font,
}
impl Field {
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::Delay => DELAY_ID,
            Self::Font => "settings-editor-font-size",
        }
    }
    fn labels(self, french: bool) -> (&'static str, &'static str) {
        if matches!(self, Self::Font) {
            return if french {
                ("Taille de police de l’éditeur", "En pixels (VS Code : 14)")
            } else {
                ("Editor font size", "In pixels (VS Code: 14)")
            };
        }
        if french {
            (
                "Enregistrer après",
                "Millisecondes sans saisie (250 à 10000)",
            )
        } else {
            ("Save after", "Milliseconds without typing (250 to 10000)")
        }
    }
    pub(super) fn search_row(self, french: bool) -> SettingsSearchRow {
        let (label, hint) = self.labels(french);
        SettingsSearchRow {
            page: SettingsPageId::Editor,
            group_anchor: None,
            group_title: String::new(),
            row_id: self.id().into(),
            text: format!("{label} {hint}"),
        }
    }
}
struct Binding {
    base: u32,
    state: Entity<EditableTextState>,
    _focus: Subscription,
    _blur: Subscription,
}
pub(super) struct Controls {
    pub(super) swatches: [FocusHandle; 8],
    fields: [Option<Binding>; 2],
}
impl Controls {
    pub(super) fn new(cx: &mut Context<TesselGpuiRoot>) -> Self {
        Self {
            swatches: std::array::from_fn(|_| cx.focus_handle()),
            fields: [None, None],
        }
    }
    pub(super) fn sync(
        &mut self,
        values: Option<&crate::workspace::EditorPreferences>,
        w: &mut Window,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        for field in [Field::Delay, Field::Font] {
            let value = values.and_then(|prefs| match field {
                Field::Delay => prefs.auto_save.then_some(prefs.auto_save_delay_ms),
                Field::Font => Some(u32::from(prefs.font_size)),
            });
            let slot = &mut self.fields[field as usize];
            let Some(base) = value else {
                *slot = None;
                continue;
            };
            if slot.as_ref().is_some_and(|binding| binding.base == base) {
                continue;
            }
            let state = cx.new(|cx| {
                EditableTextState::new(StringStorage::from(base.to_string().as_str()), cx)
            });
            let focus = state.read(cx).focus_handle(cx);
            let expected = state.clone();
            let focused = cx.on_focus(&focus, w, |root, _, _| {
                root.focus_input = FocusInputTarget::None
            });
            let blur = cx.on_blur(&focus, w, move |root, w, cx| {
                finish(root, field, &expected, 0, w, cx)
            });
            *slot = Some(Binding {
                base,
                state,
                _focus: focused,
                _blur: blur,
            });
        }
    }
    pub(super) fn render(
        &self,
        field: Field,
        french: bool,
        t: &GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> AnyElement {
        let Some(binding) = &self.fields[field as usize] else {
            return div().into_any_element();
        };
        let (label, hint) = field.labels(french);
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
                text_input(field.id())
                    .debug_selector(move || field.id().into())
                    .state(binding.state.downgrade())
                    .aria_label(label)
                    .key_context("EditableText TesselSettingsNumber")
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
                        finish(root, field, &expected, step, w, cx);
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
    field: Field,
    expected: &Entity<EditableTextState>,
    step: i32,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let Some(binding) = root.settings_controls.fields[field as usize]
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
        let (min, max, step_size, fallback) = match field {
            Field::Delay => (250, 10000, 250, 1000),
            Field::Font => (6, 100, 1, 14),
        };
        let raw = s
            .as_str()
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .map(|v| v.trunc() as i32)
            .filter(|v| *v != 0)
            .unwrap_or(fallback);
        let val = match step {
            1 => (raw.clamp(0, max) / step_size + 1) * step_size,
            -1 => ((raw.clamp(1, max + step_size) - 1) / step_size) * step_size,
            _ => raw,
        }
        .clamp(min, max) as u32;
        let canonical = val.to_string();
        if canonical != s.as_str() {
            let len = s.as_str().encode_utf16().count();
            s.replace_text_in_range(Some(0..len), &canonical, w, cx);
        }
        Some(val)
    });
    if let Some(number) = value
        && number != base
    {
        binding.base = number;
        root.dispatch_action(
            match field {
                Field::Delay => GpuiAction::SetAutoSaveDelay {
                    base,
                    delay: number,
                },
                Field::Font => GpuiAction::SetEditorFontSize { base, size: number },
            },
            cx,
        );
    }
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    #[gpui::test]
    fn editor_font_size_field_is_searchable_without_auto_save(cx: &mut gpui::TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
            root.app
                .apply_gpui_action(GpuiAction::SetLanguage { french: true });
            root.app
                .apply_gpui_action(GpuiAction::OpenSettingsPage("general".into()));
            root.app
                .apply_gpui_action(GpuiAction::SetSettingsSearch("pixels police".into()));
            root
        });
        view.update(cx, |_, w, _| w.activate()).unwrap();
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        assert!(visual.debug_bounds(DELAY_ID).is_none());
        let point = visual.debug_bounds(Field::Font.id()).unwrap().center();
        visual.simulate_click(point, Default::default());
        for (raw, expected) in [("3", 6), ("105", 100), ("", 14)] {
            view.update(cx, |root, w, cx| {
                let field = root.settings_controls.fields[Field::Font as usize]
                    .as_ref()
                    .unwrap();
                assert!(field.state.read(cx).focus_handle(cx).is_focused(w));
                field.state.update(cx, |s, cx| {
                    let len = s.as_str().len();
                    s.replace_text_in_range(Some(0..len), raw, w, cx);
                })
            })
            .unwrap();
            cx.simulate_keystrokes(view.into(), "enter");
            cx.refresh().unwrap();
            view.update(cx, |root, _, _| {
                assert_eq!(
                    root.app.gpui_snapshot(960., 640., 4.).editor_font_size,
                    expected
                )
            })
            .unwrap();
        }
    }

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
                .debug_bounds(DELAY_ID)
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
        let point = visual.debug_bounds(DELAY_ID).unwrap().center();
        visual.simulate_click(point, Default::default());
        let set = |cx: &mut gpui::TestAppContext, raw: &str| {
            view.update(cx, |root, w, cx| {
                let d = root.settings_controls.fields[Field::Delay as usize]
                    .as_ref()
                    .unwrap();
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
                let d = root.settings_controls.fields[Field::Delay as usize]
                    .as_ref()
                    .unwrap();
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
