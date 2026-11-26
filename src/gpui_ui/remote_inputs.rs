//! Native editing state stays bound to a specific remote host and file.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::{
    app::gpui_bridge::GpuiAction,
    remote_panel::control::{Command, Snapshot},
};
use gpui::{App, AppContext, Context, Entity, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};

struct Binding {
    scope: (String, String, String),
    synced: String,
    reset: bool,
    replayed: Option<(String, String)>,
    state: Entity<EditableTextState>,
    _changed: Subscription,
}

#[derive(Default)]
pub(crate) struct Inputs {
    fields: Vec<(FocusInputTarget, Binding)>,
}

fn value(
    snapshot: &Snapshot,
    target: FocusInputTarget,
) -> Option<((String, String, String), &str)> {
    let host = snapshot.connected.clone()?;
    let (key, text) = match target {
        FocusInputTarget::RemoteName => (snapshot.path.clone(), snapshot.new_name.as_str()),
        FocusInputTarget::RemoteRename => {
            let (path, text) = snapshot.rename.as_ref()?;
            (path.clone(), text.as_str())
        }
        FocusInputTarget::RemoteText => {
            let (path, text, _) = snapshot.document.as_ref()?;
            (path.clone(), text.as_deref()?)
        }
        _ => return None,
    };
    Some(((host, snapshot.path.clone(), key), text))
}

impl Inputs {
    pub fn discard_text(&mut self) {
        for (target, binding) in &mut self.fields {
            if *target == FocusInputTarget::RemoteText {
                binding.reset = true;
            }
        }
    }
    pub fn acknowledge(&mut self, snapshot: &Snapshot) {
        for (target, binding) in &mut self.fields {
            if let Some((scope, text)) = value(snapshot, *target)
                && scope == binding.scope
            {
                binding.synced = text.to_owned();
            }
        }
    }
    pub fn sync(
        &mut self,
        snapshot: &Snapshot,
        window: &mut Window,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> Vec<Command> {
        let mut pending = Vec::new();
        for target in [
            FocusInputTarget::RemoteName,
            FocusInputTarget::RemoteRename,
            FocusInputTarget::RemoteText,
        ] {
            let next = value(snapshot, target);
            let mut restore_focus = false;
            let keep = self.fields.iter_mut().any(|(kind, binding)| {
                if *kind != target {
                    return false;
                }
                restore_focus = binding.state.read(cx).focus_handle(cx).is_focused(window);
                next.as_ref().is_some_and(|(scope, text)| {
                    if binding.scope != *scope {
                        restore_focus = false;
                        return false;
                    }
                    if binding.reset {
                        return false;
                    }
                    let local = binding.state.read(cx).as_str();
                    if local == *text {
                        binding.synced = (*text).to_owned();
                        binding.replayed = None;
                        return true;
                    }
                    let limit = if target == FocusInputTarget::RemoteText {
                        2 * 1024 * 1024
                    } else {
                        255
                    };
                    if binding.synced == *text && local.len() <= limit {
                        let already_replayed = binding
                            .replayed
                            .as_ref()
                            .is_some_and(|(sent, base)| sent == local && base == *text);
                        if !snapshot.busy && !already_replayed {
                            binding.replayed = Some((local.to_owned(), (*text).to_owned()));
                            pending.push(match target {
                                FocusInputTarget::RemoteName => Command::Name(local.to_owned()),
                                FocusInputTarget::RemoteRename => {
                                    Command::RenameDraft(local.to_owned())
                                }
                                _ => Command::Edit(local.to_owned()),
                            });
                        }
                        return true;
                    }
                    false
                })
            });
            if keep {
                continue;
            }
            self.fields.retain(|(kind, _)| *kind != target);
            let Some((scope, text)) = next else {
                continue;
            };
            let state = cx.new(|cx| EditableTextState::new(StringStorage::from(text), cx));
            if restore_focus {
                window.focus(&state.read(cx).focus_handle(cx), cx);
            }
            let expected = scope.clone();
            let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
                // Old queued edits must never reach a new tab, host, or discarded document.
                if !root
                    .remote_inputs
                    .fields
                    .iter()
                    .any(|(kind, binding)| *kind == target && binding.state == state)
                    || root.app.gpui_has_open_modal()
                {
                    return;
                }
                let Some(snapshot) = root.app.gpui_remote_input_snapshot() else {
                    return;
                };
                if snapshot.busy {
                    return;
                }
                let Some((scope, before)) = value(&snapshot, target) else {
                    return;
                };
                let text = state.read(cx).as_str().to_owned();
                if scope != expected || before == text {
                    return;
                }
                root.dispatch(
                    GpuiAction::Remote(match target {
                        FocusInputTarget::RemoteName => Command::Name(text),
                        FocusInputTarget::RemoteRename => Command::RenameDraft(text),
                        _ => Command::Edit(text),
                    }),
                    cx,
                );
            });
            self.fields.push((
                target,
                Binding {
                    scope,
                    synced: text.to_owned(),
                    reset: false,
                    replayed: None,
                    state,
                    _changed: changed,
                },
            ));
        }
        pending
    }

    pub fn get(&self, target: FocusInputTarget) -> Option<&Entity<EditableTextState>> {
        self.fields
            .iter()
            .find(|(kind, _)| *kind == target)
            .map(|(_, binding)| &binding.state)
    }

    pub fn focused(&self, window: &Window, cx: &App) -> Option<FocusInputTarget> {
        self.fields
            .iter()
            .find(|(_, binding)| binding.state.read(cx).focus_handle(cx).is_focused(window))
            .map(|(target, _)| *target)
    }
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use gpui::{EntityInputHandler, TestAppContext};

    #[gpui::test]
    fn remote_image_right_click_opens_download_menu(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
            root.app.install_remote_image_fixture();
            root
        });
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let rect = visual
            .debug_bounds("remote-preview-pixels")
            .expect("SSH image");
        let pane = visual.debug_bounds("remote-image-viewport").unwrap();
        let width = f32::from(rect.size.width);
        let height = f32::from(rect.size.height);
        assert!(width > 0. && height > 0.);
        assert!((width / height - 2.).abs() < 0.02);
        assert!((f32::from(rect.center().x - pane.center().x)).abs() <= 1.);
        assert!((f32::from(rect.center().y - pane.center().y)).abs() <= 1.);
        let expected = 320_f32
            .min(f32::from(pane.size.width) - 40.)
            .min((f32::from(pane.size.height) - 40.) * 2.);
        assert!((width - expected).abs() <= 1.);
        let pos = rect.center();
        visual.simulate_mouse_down(pos, gpui::MouseButton::Right, Default::default());
        visual.simulate_mouse_up(pos, gpui::MouseButton::Right, Default::default());
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        assert!(visual.debug_bounds("remote-image-menu").is_some());
        visual.simulate_click(gpui::point(gpui::px(5.), gpui::px(5.)), Default::default());
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        assert!(visual.debug_bounds("remote-image-menu").is_none());
    }

    #[gpui::test]
    fn remote_native_input_updates_current_document_and_drops_old_bindings(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|window, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().into()), false, cx);
            root.app.install_remote_input_fixture();
            let snapshot = root.app.gpui_remote_input_snapshot().unwrap();
            root.remote_inputs.sync(&snapshot, window, cx);
            root
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let input = view
            .update(cx, |root, _, _| {
                root.remote_inputs
                    .get(FocusInputTarget::RemoteText)
                    .unwrap()
                    .clone()
            })
            .unwrap();
        view.update(cx, |_, window, cx| {
            input.update(cx, |state, cx| {
                state.replace_text_in_range(Some(0..9), "É😀", window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, window, cx| {
            let snapshot = root.app.gpui_remote_input_snapshot().unwrap();
            assert!(
                snapshot
                    .document
                    .unwrap()
                    .1
                    .unwrap()
                    .starts_with("É😀 remote text")
            );
            let snapshot = root.app.gpui_remote_input_snapshot().unwrap();
            root.remote_inputs.sync(&snapshot, window, cx);
            assert_eq!(
                root.remote_inputs.get(FocusInputTarget::RemoteText),
                Some(&input)
            );
            // Discard replaces the entity and its undo history, not just its display.
            root.dispatch(GpuiAction::Remote(Command::Discard), cx);
            let snapshot = root.app.gpui_remote_input_snapshot().unwrap();
            root.remote_inputs.sync(&snapshot, window, cx);
            assert_ne!(
                root.remote_inputs.get(FocusInputTarget::RemoteText),
                Some(&input)
            );
        })
        .unwrap();
        view.update(cx, |_, _, cx| {
            input.update(cx, |state, cx| state.emplace("stale queued edit", cx))
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            let snapshot = root.app.gpui_remote_input_snapshot().unwrap();
            assert!(!snapshot.dirty);
            assert!(
                snapshot
                    .document
                    .unwrap()
                    .1
                    .unwrap()
                    .starts_with("Synthetic remote text")
            );
        })
        .unwrap();
        let current = view
            .update(cx, |root, window, cx| {
                let current = root
                    .remote_inputs
                    .get(FocusInputTarget::RemoteText)
                    .unwrap()
                    .clone();
                window.focus(&current.read(cx).focus_handle(cx), cx);
                current
            })
            .unwrap();
        let release = view
            .update(cx, |root, _, _| root.app.hold_remote_input_fixture())
            .unwrap();
        view.update(cx, |_, _, cx| {
            current.update(cx, |state, cx| state.emplace("pending during save 😀", cx));
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, window, cx| {
            let snapshot = root.app.gpui_remote_input_snapshot().unwrap();
            assert!(snapshot.busy);
            assert!(root.remote_inputs.sync(&snapshot, window, cx).is_empty());
            assert_eq!(
                root.remote_inputs.get(FocusInputTarget::RemoteText),
                Some(&current)
            );
            assert!(current.read(cx).focus_handle(cx).is_focused(window));
        })
        .unwrap();
        release();
        view.update(cx, |root, window, cx| {
            root.app.poll_for_gpui();
            let snapshot = root.app.gpui_remote_input_snapshot().unwrap();
            assert!(!snapshot.busy);
            let queued = root.remote_inputs.sync(&snapshot, window, cx);
            assert_eq!(queued.len(), 1);
            assert!(root.remote_inputs.sync(&snapshot, window, cx).is_empty());
            for command in queued {
                root.dispatch(GpuiAction::Remote(command), cx);
            }
            let snapshot = root.app.gpui_remote_input_snapshot().unwrap();
            assert_eq!(
                snapshot.document.unwrap().1.unwrap(),
                "pending during save 😀"
            );
            assert!(current.read(cx).focus_handle(cx).is_focused(window));
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |_, window, cx| {
            current.update(cx, |state, cx| {
                state.replace_and_mark_text_in_range(Some(0..7), "日", None, window, cx);
                assert_eq!(state.marked_text_range(window, cx), Some(0..1));
                state.replace_and_mark_text_in_range(None, "日本", None, window, cx);
                assert_eq!(state.marked_text_range(window, cx), Some(0..2));
                state.replace_text_in_range(None, "日本語", window, cx);
                assert!(state.marked_text_range(window, cx).is_none());
                state.set_selected_text_range(0..3, window, cx);
                state.paste(gpui::ClipboardItem::new_string("É😀".into()), window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        let expected = "É😀 during save 😀";
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app
                    .gpui_remote_input_snapshot()
                    .unwrap()
                    .document
                    .unwrap()
                    .1
                    .unwrap(),
                expected
            );
        })
        .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_ne!(
                root.app
                    .gpui_remote_input_snapshot()
                    .unwrap()
                    .document
                    .unwrap()
                    .1
                    .unwrap(),
                expected
            );
        })
        .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app
                    .gpui_remote_input_snapshot()
                    .unwrap()
                    .document
                    .unwrap()
                    .1
                    .unwrap(),
                expected
            );
        })
        .unwrap();
    }
}
