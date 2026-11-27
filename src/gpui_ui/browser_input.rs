//! Native address editing; never an IPC or navigation-policy bypass.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::app::gpui_bridge::{AddressCommand, GpuiAction};
use gpui::{App, AppContext, Context, Entity, EntityInputHandler, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
type Scope = (u64, Option<u64>);
const LIMIT: usize = 8192;
struct Binding {
    scope: Scope,
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Input {
    binding: Option<Binding>,
    side: bool,
    pub select_all: bool,
}
gpui::actions!(
    tessel_browser_address,
    [Navigate, Dismiss, Back, Forward, Reload, HardReload, Find]
);
fn input(root: &TesselGpuiRoot, side: bool) -> &Input {
    if side {
        &root.side_browser_input
    } else {
        &root.browser_input
    }
}
fn input_mut(root: &mut TesselGpuiRoot, side: bool) -> &mut Input {
    if side {
        &mut root.side_browser_input
    } else {
        &mut root.browser_input
    }
}
impl Input {
    pub fn side() -> Self {
        Self {
            side: true,
            ..Default::default()
        }
    }
    pub fn focused(&self, window: &Window, cx: &App) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|b| b.state.read(cx).focus_handle(cx).is_focused(window))
    }
    pub fn state(&self) -> Option<&Entity<EditableTextState>> {
        self.binding.as_ref().map(|b| &b.state)
    }
    pub fn sync(&mut self, scope: Scope, text: &str, cx: &mut Context<TesselGpuiRoot>) {
        if self.binding.as_ref().is_some_and(|b| {
            b.scope == scope
                && (b.state.read(cx).as_str() == text || (b.too_long && b.synced == text))
        }) {
            return;
        }
        let state = cx.new(|cx| EditableTextState::new(StringStorage::from(text), cx));
        let side = self.side;
        let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
            let Some(b) = input(root, side).binding.as_ref() else {
                return;
            };
            if b.scope != scope
                || b.state != state
                || root.app.browser_address_for(side) != Some((scope, b.synced.as_str()))
            {
                return;
            }
            let base = b.synced.clone();
            let too_long = state.read(cx).as_str().len() > LIMIT;
            input_mut(root, side).binding.as_mut().unwrap().too_long = too_long;
            if !too_long {
                root.dispatch(
                    GpuiAction::NativeBrowserAddress {
                        scope,
                        base,
                        text: state.read(cx).as_str().to_owned(),
                        navigate: false,
                    },
                    cx,
                );
                acknowledge(root, side);
            }
            cx.notify();
        });
        self.binding = Some(Binding {
            scope,
            state,
            synced: text.to_owned(),
            too_long: false,
            _changed: changed,
        });
    }
    pub fn render(
        &self,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder, px,
        };
        let Some(b) = &self.binding else {
            return div().flex_1().into_any_element();
        };
        let side = self.side;
        let expected = b.state.clone();
        let clicked = b.state.clone();
        let dismissed = b.state.clone();
        let field = gpui_ce_elements::editable_text::text_input(if side {
            "native-side-browser-address"
        } else {
            "native-browser-address"
        })
        .state(b.state.downgrade())
        .key_context("EditableText TesselBrowserAddress")
        .on_action(cx.listener(move |root, _: &Navigate, window, cx| {
            if input(root, side).state() == Some(&expected) {
                navigate_for(root, side, window, cx);
            }
        }))
        .on_action(cx.listener(move |root, _: &Dismiss, window, cx| {
            if input(root, side).state() == Some(&dismissed) {
                dismiss(root, side, window, cx);
            }
        }))
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |root, _, _, cx| {
                if input(root, side).state() == Some(&clicked)
                    && root.app.browser_address_for(side).is_some()
                {
                    begin_edit_for(root, side, cx);
                    root.focus_input = FocusInputTarget::BrowserUrl;
                }
                cx.stop_propagation();
            }),
        )
        .w_full()
        .h(px(26.))
        .px(px(8.))
        .py(px(3.))
        .text_size(px(11.5))
        .text_color(tokens.text_strong);
        let scope = b.scope;
        let state = b.state.clone();
        let command = move |root: &mut TesselGpuiRoot,
                            command,
                            window: &mut Window,
                            cx: &mut Context<TesselGpuiRoot>| {
            if input(root, side).state() == Some(&state)
                && !state.update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
            {
                root.dispatch(GpuiAction::BrowserAddressCommand { scope, command }, cx);
            }
            cx.stop_propagation();
        };
        let find = command.clone();
        let back = command.clone();
        let forward = command.clone();
        let reload = command.clone();
        div()
            .debug_selector(move || {
                if side {
                    "side-browser-address".into()
                } else {
                    "center-browser-address".into()
                }
            })
            .on_action(cx.listener(move |root, _: &Find, window, cx| {
                find(root, AddressCommand::Find, window, cx)
            }))
            .on_action(cx.listener(move |root, _: &Back, window, cx| {
                back(root, AddressCommand::Back, window, cx)
            }))
            .on_action(cx.listener(move |root, _: &Forward, window, cx| {
                forward(root, AddressCommand::Forward, window, cx)
            }))
            .on_action(cx.listener(move |root, _: &Reload, window, cx| {
                reload(root, AddressCommand::Reload, window, cx)
            }))
            .on_action(cx.listener(move |root, _: &HardReload, window, cx| {
                command(root, AddressCommand::HardReload, window, cx)
            }))
            .flex_1()
            .min_w_0()
            .bg(tokens.term)
            .rounded(px(4.))
            .border_1()
            .border_color(tokens.border_strong)
            .child(field)
            .when(b.too_long, |el| {
                el.child(
                    div()
                        .text_size(px(10.))
                        .text_color(tokens.warn)
                        .child(if french {
                            "Adresse limitée à 8 Kio"
                        } else {
                            "Address limit: 8 KiB"
                        }),
                )
            })
            .into_any_element()
    }
}
pub(crate) fn begin_edit_for(
    root: &mut TesselGpuiRoot,
    side: bool,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if let Some((scope, _)) = root.app.browser_address_for(side) {
        root.dispatch(GpuiAction::BeginNativeBrowserAddress(scope), cx);
    }
}
fn dismiss(
    root: &mut TesselGpuiRoot,
    side: bool,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let Some(b) = input(root, side).binding.as_ref() else {
        return;
    };
    if root.app.browser_address_for(side) != Some((b.scope, b.synced.as_str())) {
        return;
    }
    if b.state
        .update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
    {
        return;
    }
    root.dispatch(GpuiAction::CancelNativeBrowserAddress(b.scope), cx);
    root.focus_input = FocusInputTarget::None;
    window.focus(&root.focus_handle, cx);
}
fn acknowledge(root: &mut TesselGpuiRoot, side: bool) {
    if let Some((scope, text)) = root
        .app
        .browser_address_for(side)
        .map(|(s, t)| (s, t.to_owned()))
        && let Some(b) = input_mut(root, side).binding.as_mut()
        && b.scope == scope
    {
        b.synced = text;
    }
}
pub(crate) fn navigate_for(
    root: &mut TesselGpuiRoot,
    side: bool,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let Some(b) = input(root, side).binding.as_ref() else {
        return;
    };
    if root.app.browser_address_for(side) != Some((b.scope, b.synced.as_str())) {
        return;
    }
    let state = b.state.clone();
    if state.update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
        || state.read(cx).as_str().len() > LIMIT
    {
        return;
    }
    let scope = b.scope;
    root.dispatch(
        GpuiAction::NativeBrowserAddress {
            scope,
            base: b.synced.clone(),
            text: state.read(cx).as_str().to_owned(),
            navigate: true,
        },
        cx,
    );
    acknowledge(root, side);
    if root.app.browser_address_for(side).map(|v| v.0) != Some(scope) {
        root.focus_input = FocusInputTarget::None;
        window.focus(&root.focus_handle, cx);
    }
}

#[cfg(all(test, feature = "visual-test", windows))]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    fn bindings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
    }
    #[gpui::test]
    fn failed_page_actions_target_their_own_browser_pane(cx: &mut TestAppContext) {
        const CENTER: &str = "https://center.example/report?format=pdf&year=2025";
        const SIDE: &str = "http://localhost:5173/files/a%20b?download=1";
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.dispatch(GpuiAction::PaletteExecute("browser".into()), cx);
            if !root.app.gpui_snapshot(960., 640., 4.).toolbar.right_open {
                root.dispatch(GpuiAction::ToggleRightSidebar, cx);
            }
            root.dispatch(GpuiAction::NewSideBrowser, cx);
            root.app.browser.connection_failure_fixture(CENTER);
            let side = root.app.selected_side_browser.unwrap();
            root.app
                .side_browsers
                .get_mut(&side)
                .unwrap()
                .connection_failure_fixture(SIDE);
            root
        });
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        for (selector, expected) in [
            ("side-browser-copy-failed-address", SIDE),
            ("browser-copy-failed-address", CENTER),
        ] {
            let bounds = visual.debug_bounds(selector).unwrap();
            visual.simulate_click(bounds.center(), Default::default());
            assert_eq!(
                cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
                    .as_deref(),
                Some(expected)
            );
        }
        assert!(visual.debug_bounds("browser-try-https").is_none());
        let upgrade = visual.debug_bounds("side-browser-try-https").unwrap();
        visual.simulate_click(upgrade.center(), Default::default());
        view.update(cx, |root, _, _| {
            let id = root.app.selected_side_browser.unwrap();
            assert_eq!(
                root.app.side_browsers[&id].address_str(),
                "https://localhost:5173/files/a%20b?download=1"
            );
            assert_eq!(root.app.browser.failed_address(), Some(CENTER));
        })
        .unwrap();
    }

    #[gpui::test]
    fn new_center_page_focuses_empty_address_for_immediate_typing(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.dispatch(GpuiAction::PaletteExecute("browser".into()), cx);
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.browser_input.focused(window, cx));
            assert!(
                root.browser_input
                    .state()
                    .unwrap()
                    .read(cx)
                    .as_str()
                    .is_empty()
            );
            assert_eq!(root.app.browser.restart_location().0, "about:blank");
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "a b c");
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.browser.address_str(), "abc")
        })
        .unwrap();
    }

    #[gpui::test]
    fn side_plus_focuses_address_and_enter_only_navigates_the_selected_page(
        cx: &mut TestAppContext,
    ) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.dispatch(GpuiAction::SelectBrowserTab, cx);
            if !root.app.gpui_snapshot(960., 640., 4.).toolbar.right_open {
                root.dispatch(GpuiAction::ToggleRightSidebar, cx);
            }
            root
        });
        cx.refresh().unwrap();
        let center = view
            .update(cx, |root, _, _| root.app.browser.address_str().to_owned())
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let button = visual.debug_bounds("side-browser-new").unwrap().center();
        visual.simulate_click(button, Default::default());
        cx.refresh().unwrap();
        let (id, scope) = view
            .update(cx, |root, w, cx| {
                assert!(
                    root.app.selected_side_browser.is_some(),
                    "new-page button must be reachable at normal panel width"
                );
                assert!(root.side_browser_input.focused(w, cx));
                let id = root.app.selected_side_browser.unwrap();
                let (scope, text) = root.app.browser_address_for(true).unwrap();
                assert!(text.is_empty());
                root.side_browser_input
                    .state()
                    .unwrap()
                    .clone()
                    .update(cx, |s, cx| {
                        s.replace_text_in_range(None, "example.test/side", w, cx)
                    });
                (id, scope)
            })
            .unwrap();
        cx.run_until_parked();
        cx.dispatch_action(view.into(), Navigate);
        cx.run_until_parked();
        view.update(cx, |root, _, cx| {
            assert_eq!(
                root.app.side_browsers[&id].address_str(),
                "https://example.test/side"
            );
            assert_eq!(root.app.browser.address_str(), center);
            root.dispatch(GpuiAction::NewSideBrowser, cx);
            root.dispatch(
                GpuiAction::NativeBrowserAddress {
                    scope,
                    base: String::new(),
                    text: "https://wrong.test/".into(),
                    navigate: true,
                },
                cx,
            );
            assert_eq!(root.app.browser_address_for(true).unwrap().1, "");
            assert_eq!(root.app.side_browsers.len(), 2);
        })
        .unwrap();
    }

    #[gpui::test]
    fn native_address_unicode_undo_pending_navigation_and_policy(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            let project = dir.path().join("project");
            std::fs::create_dir_all(&project).unwrap();
            root.dispatch(GpuiAction::AddProjectPath(project), cx);
            root.dispatch(GpuiAction::SelectBrowserTab, cx);
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| w.focus(&root.focus_handle, cx))
            .unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-l");
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, w, cx| {
                let input = root.browser_input.state().unwrap().clone();
                assert!(input.read(cx).focus_handle(cx).is_focused(w));
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "al😀ha", w, cx);
                    s.set_selected_text_range(2..4, w, cx);
                    s.paste(gpui::ClipboardItem::new_string("p".into()), w, cx);
                });
                input
            })
            .unwrap();
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.app.browser.address_str(), "alpha");
            input.update(cx, |s, cx| {
                s.replace_text_in_range(Some(0..5), "example.test/path", w, cx)
            });
            navigate_for(root, false, w, cx);
            assert_eq!(root.app.browser.address_str(), "https://example.test/path");
            assert!(!root.app.browser.is_started());
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            let current = root.browser_input.state().unwrap().clone();
            assert_ne!(current, input);
            current.update(cx, |s, cx| {
                let n = s.as_str().encode_utf16().count();
                s.replace_text_in_range(Some(0..n), "javascript:alert(1)", w, cx);
            });
            navigate_for(root, false, w, cx);
            assert!(!root.app.browser.error_message().is_empty());
            assert!(!root.app.browser.is_started());
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_address_rejects_stale_navigation_and_hidden_modal_edits(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            let project = dir.path().join("project");
            std::fs::create_dir_all(&project).unwrap();
            root.dispatch(GpuiAction::AddProjectPath(project), cx);
            root.dispatch(GpuiAction::SelectBrowserTab, cx);
            root
        });
        cx.refresh().unwrap();
        let old = view
            .update(cx, |root, _, _| root.browser_input.state().unwrap().clone())
            .unwrap();
        view.update(cx, |root, w, cx| {
            root.dispatch(
                GpuiAction::BrowserSetAddress("https://new.example/".into()),
                cx,
            );
            old.update(cx, |s, cx| s.replace_text_in_range(None, "late", w, cx));
            navigate_for(root, false, w, cx);
            assert_eq!(root.app.browser.address_str(), "https://new.example/");
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            let current = root.browser_input.state().unwrap().clone();
            let scope = root.app.gpui_browser_address().unwrap().0;
            root.dispatch(GpuiAction::CloseBrowserTab, cx);
            let project = dir.path().join("project");
            std::fs::create_dir_all(&project).unwrap();
            root.dispatch(GpuiAction::AddProjectPath(project), cx);
            root.dispatch(GpuiAction::SelectBrowserTab, cx);
            assert_ne!(root.app.gpui_browser_address().unwrap().0, scope);
            current.update(cx, |s, cx| s.replace_text_in_range(None, "closed", w, cx));
            navigate_for(root, false, w, cx);
            assert_eq!(root.app.browser.address_str(), "https://new.example/");
            root.dispatch(GpuiAction::OpenHelp, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert!(!root.browser_input.focused(w, cx));
            let current = root.browser_input.state().unwrap().clone();
            current.update(cx, |s, cx| s.replace_text_in_range(None, "modal", w, cx));
            navigate_for(root, false, w, cx);
            assert_eq!(root.app.browser.address_str(), "https://new.example/");
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.browser.address_str(), "https://new.example/")
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_address_retains_oversize_and_defers_ime(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            let project = dir.path().join("project");
            std::fs::create_dir_all(&project).unwrap();
            root.dispatch(GpuiAction::AddProjectPath(project), cx);
            root.dispatch(GpuiAction::SelectBrowserTab, cx);
            root
        });
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, w, cx| {
                let input = root.browser_input.state().unwrap().clone();
                input.update(cx, |s, cx| {
                    let n = s.as_str().encode_utf16().count();
                    s.replace_text_in_range(Some(0..n), &"x".repeat(LIMIT + 1), w, cx);
                });
                input
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.browser_input.state(), Some(&input));
            assert!(root.browser_input.binding.as_ref().unwrap().too_long);
            navigate_for(root, false, w, cx);
            assert_eq!(root.app.browser.address_str(), "https://example.com");
            input.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(Some(0..LIMIT + 1), "日", None, w, cx)
            });
            let generation = root.app.browser.address_generation();
            navigate_for(root, false, w, cx);
            assert_eq!(root.app.browser.address_generation(), generation);
            input.update(cx, |s, cx| {
                s.replace_text_in_range(None, "example.test", w, cx)
            });
            navigate_for(root, false, w, cx);
            assert_eq!(root.app.browser.address_str(), "https://example.test/");
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_address_keeps_pending_draft_across_page_navigation_and_escape_restores_location(
        cx: &mut TestAppContext,
    ) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            let project = dir.path().join("project");
            std::fs::create_dir_all(&project).unwrap();
            root.dispatch(GpuiAction::AddProjectPath(project), cx);
            root.dispatch(GpuiAction::SelectBrowserTab, cx);
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| w.focus(&root.focus_handle, cx))
            .unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-l");
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, w, cx| {
                let input = root.browser_input.state().unwrap().clone();
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "example.test/draft", w, cx)
                });
                // Deliver a real browser mailbox event BEFORE TextChanged is drained.
                root.app
                    .browser
                    .navigation_fixture("https://redirect.example/one");
                navigate_for(root, false, w, cx);
                assert_eq!(root.app.browser.address_str(), "https://example.test/draft");
                input
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-l");
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            let current = root.browser_input.state().unwrap().clone();
            assert_ne!(current, input);
            current.update(cx, |s, cx| {
                s.replace_text_in_range(None, "unfinished", w, cx)
            });
            root.app
                .browser
                .navigation_fixture("https://redirect.example/two");
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, _, cx| {
            assert_eq!(
                root.browser_input.state().unwrap().read(cx).as_str(),
                "unfinished"
            )
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "escape");
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(
                root.app.browser.address_str(),
                "https://redirect.example/two"
            );
            assert!(!root.browser_input.focused(w, cx));
        })
        .unwrap();
    }
}
