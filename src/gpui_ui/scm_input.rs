//! Commit drafts retain native history per project/path and never submit a stale panel.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::{
    app::gpui_bridge::{GpuiAction, NativeScmScope},
    git_result::Owner,
};
use gpui::{App, AppContext, Context, Entity, EntityInputHandler, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;
const LIMIT: usize = 65536;
struct Binding {
    scope: NativeScmScope,
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    dirty_marked: bool,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Input {
    bindings: HashMap<Owner, Binding>,
    active: Option<NativeScmScope>,
}
gpui::actions!(tessel_scm, [Submit, Dismiss]);
impl Input {
    pub fn focused(&self, w: &Window, cx: &App) -> bool {
        self.bindings
            .values()
            .any(|b| b.state.read(cx).focus_handle(cx).is_focused(w))
    }
    pub fn state(&self) -> Option<&Entity<EditableTextState>> {
        let scope = self.active.as_ref()?;
        self.bindings
            .get(&scope.owner)
            .filter(|b| b.scope == *scope)
            .map(|b| &b.state)
    }
    pub fn retain(&mut self, ids: &[u64]) {
        self.bindings.retain(|_, b| ids.contains(&b.scope.id));
    }
    pub fn sync(&mut self, scope: NativeScmScope, text: &str, cx: &mut Context<TesselGpuiRoot>) {
        self.active = Some(scope.clone());
        if self.bindings.get(&scope.owner).is_some_and(|b| {
            b.scope == scope
                && (b.state.read(cx).as_str() == text || (b.too_long && b.synced == text))
        }) {
            return;
        }
        let state = cx.new(|cx| EditableTextState::new(StringStorage::from(text), cx));
        let captured = scope.clone();
        let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
            if root
                .scm_input
                .bindings
                .get(&captured.owner)
                .is_some_and(|b| b.scope == captured && b.state == state)
                && root.app.gpui_scm_input().map(|v| v.0) == Some(captured.clone())
            {
                flush(root, cx);
                cx.notify();
            }
        });
        self.bindings.insert(
            scope.owner.clone(),
            Binding {
                scope,
                state,
                synced: text.to_owned(),
                too_long: false,
                dirty_marked: false,
                _changed: changed,
            },
        );
    }
    pub fn render(
        &self,
        busy: bool,
        can_submit: bool,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
            div, prelude::FluentBuilder, px,
        };
        let Some(scope) = &self.active else {
            return div().into_any_element();
        };
        let Some(b) = self
            .bindings
            .get(&scope.owner)
            .filter(|b| b.scope == *scope)
        else {
            return div().into_any_element();
        };
        let send = b.state.clone();
        let dismiss_state = b.state.clone();
        let clicked = b.state.clone();
        let button = b.state.clone();
        let disabled =
            busy || !can_submit || b.too_long || b.state.read(cx).as_str().trim().is_empty();
        let field = gpui_ce_elements::editable_text::text_area("native-scm-message")
            .state(b.state.downgrade())
            .accepts_input(!busy)
            .key_context("EditableText TesselScm")
            .placeholder(if french {
                "Message de commit…"
            } else {
                "Commit message…"
            })
            .on_action(cx.listener(move |root, _: &Submit, w, cx| submit(root, &send, w, cx)))
            .on_action(cx.listener(move |root, _: &Dismiss, w, cx| {
                if matches(root, &dismiss_state)
                    && !dismiss_state.update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
                {
                    root.focus_input = FocusInputTarget::None;
                    w.focus(&root.focus_handle, cx);
                }
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |root, _, _, cx| {
                    if matches(root, &clicked) {
                        root.focus_input = FocusInputTarget::ScmCommitMessage;
                    }
                    cx.stop_propagation();
                }),
            )
            .w_full()
            .min_h(px(55.))
            .max_h(px(140.))
            .overflow_y_scroll()
            .px(px(8.))
            .py(px(6.))
            .text_size(px(12.))
            .text_color(tokens.text);
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .bg(tokens.term)
                    .border_1()
                    .border_color(tokens.border)
                    .rounded(px(4.))
                    .child(field),
            )
            .when(b.too_long, |el| {
                el.child(
                    div()
                        .text_size(px(10.))
                        .text_color(tokens.warn)
                        .child(if french {
                            "Message limité à 64 Kio"
                        } else {
                            "Message limit: 64 KiB"
                        }),
                )
            })
            .child(
                div()
                    .id("native-scm-commit")
                    .px(px(10.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .bg(if disabled {
                        tokens.surface_2
                    } else {
                        tokens.accent
                    })
                    .text_color(if disabled {
                        tokens.text_dim
                    } else {
                        tokens.accent_fg
                    })
                    .when(!disabled, |el| {
                        el.cursor_pointer().on_click(
                            cx.listener(move |root, _, w, cx| submit(root, &button, w, cx)),
                        )
                    })
                    .child(if busy {
                        if french { "En cours…" } else { "Working…" }
                    } else {
                        "Commit"
                    }),
            )
            .into_any_element()
    }
}
/// Flush before polling Git and before workspace actions, so a queued native edit
/// cannot be erased by the completion of the previous message.
pub(crate) fn flush(root: &mut TesselGpuiRoot, cx: &mut Context<TesselGpuiRoot>) -> bool {
    let Some((scope, base)) = root.app.gpui_scm_input() else {
        return false;
    };
    let Some(b) = root
        .scm_input
        .bindings
        .get(&scope.owner)
        .filter(|b| b.scope == scope && b.synced == base)
    else {
        return false;
    };
    let state = b.state.clone();
    let marked = b.dirty_marked;
    let too_long = state.read(cx).as_str().len() > LIMIT;
    if too_long {
        if !marked {
            root.dispatch(
                GpuiAction::NativeScmTouch {
                    scope: scope.clone(),
                    base: base.to_owned(),
                },
                cx,
            );
        }
        if let Some(b) = root.scm_input.bindings.get_mut(&scope.owner) {
            b.too_long = true;
            b.dirty_marked = true;
        }
        return false;
    }
    if state.read(cx).as_str() != base {
        root.dispatch(
            GpuiAction::NativeScmEdit {
                scope: scope.clone(),
                base: base.to_owned(),
                text: state.read(cx).as_str().to_owned(),
            },
            cx,
        );
    }
    if let Some((current, text)) = root.app.gpui_scm_input()
        && current == scope
        && let Some(b) = root.scm_input.bindings.get_mut(&scope.owner)
    {
        if b.synced != text {
            b.synced = text.to_owned();
        }
        b.too_long = false;
        b.dirty_marked = false;
    }
    true
}
fn matches(root: &TesselGpuiRoot, expected: &Entity<EditableTextState>) -> bool {
    root.scm_input.state() == Some(expected)
        && root
            .scm_input
            .active
            .as_ref()
            .is_some_and(|scope| root.app.gpui_scm_input().map(|v| v.0) == Some(scope.clone()))
}
pub(crate) fn submit(
    root: &mut TesselGpuiRoot,
    expected: &Entity<EditableTextState>,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if !matches(root, expected) || expected.update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
    {
        return;
    }
    if !flush(root, cx) || !root.app.gpui_scm_can_submit() {
        return;
    }
    if let Some((scope, text)) = root.app.gpui_scm_input().map(|(s, t)| (s, t.to_owned())) {
        root.dispatch(GpuiAction::NativeScmSubmit { scope, base: text }, cx);
    }
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use crate::workspace::SideTab;
    use gpui::TestAppContext;
    fn bindings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
    }
    fn prepare(
        root: &mut TesselGpuiRoot,
        project: std::path::PathBuf,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        std::fs::create_dir_all(&project).unwrap();
        root.dispatch(GpuiAction::AddProjectPath(project), cx);
        root.dispatch(GpuiAction::SelectSideTab(SideTab::Changes), cx);
        root.focus_input = FocusInputTarget::ScmCommitMessage;
    }
    #[gpui::test]
    fn native_scm_enter_is_newline_and_ime_prevents_submit(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            prepare(&mut root, dir.path().join("project"), cx);
            root
        });
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, w, cx| {
                let input = root.scm_input.state().unwrap().clone();
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
        view.update(cx, |_, w, cx| {
            input.update(cx, |s, cx| s.set_selected_text_range(5..5, w, cx))
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "enter");
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.app.gpui_scm_draft().unwrap().2, "alpha\n");
            assert!(!root.app.gpui_scm_busy());
            input.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "日", None, w, cx)
            });
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-enter");
        view.update(cx, |root, _, _| assert!(!root.app.gpui_scm_busy()))
            .unwrap();
    }
    #[gpui::test]
    fn native_scm_completion_preserves_pending_and_oversize_edits(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            prepare(&mut root, dir.path().join("project"), cx);
            root.dispatch(GpuiAction::ScmSetCommitMessage("submitted".into()), cx);
            root
        });
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, w, cx| {
                let input = root.scm_input.state().unwrap().clone();
                let tx = root.app.install_scm_pending_fixture();
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(Some(0..9), "new draft", w, cx)
                });
                tx.send(Ok("done".into())).unwrap();
                root.poll_backend(cx);
                assert_eq!(root.app.gpui_scm_draft().unwrap().2, "new draft");
                let tx = root.app.install_scm_pending_fixture();
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(Some(0..9), &"x".repeat(LIMIT + 1), w, cx)
                });
                tx.send(Ok("done".into())).unwrap();
                root.poll_backend(cx);
                input
            })
            .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.scm_input.state(), Some(&input));
            assert_eq!(input.read(cx).as_str().len(), LIMIT + 1);
            input.update(cx, |s, cx| {
                s.replace_text_in_range(Some(0..LIMIT + 1), "retry", w, cx)
            });
            flush(root, cx);
            let tx = root.app.install_scm_pending_fixture();
            tx.send(Err("synthetic refusal".into())).unwrap();
            root.poll_backend(cx);
            assert_eq!(root.app.gpui_scm_draft().unwrap().2, "retry");
            let tx = root.app.install_scm_pending_fixture();
            tx.send(Ok("Error: literal successful summary".into()))
                .unwrap();
            root.poll_backend(cx);
            assert_eq!(root.app.gpui_scm_draft().unwrap().2, "");
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_ne!(root.scm_input.state(), Some(&input))
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_scm_drafts_stay_with_project_and_old_submit_is_rejected(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let second = dir.path().join("second");
        std::fs::create_dir(&second).unwrap();
        crate::git::run(&second, &["-c", "init.templateDir=", "init", "-q"], None).unwrap();
        for (key, value) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.test"),
            ("commit.gpgsign", "false"),
            ("core.fsmonitor", "false"),
            ("core.hooksPath", ".fixture-empty-hooks"),
        ] {
            crate::git::run(&second, &["config", "--local", key, value], None).unwrap();
        }
        std::fs::write(second.join("file.txt"), "synthetic\n").unwrap();
        crate::git::stage_files(&second, &["file.txt".into()]).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            prepare(&mut root, dir.path().join("first"), cx);
            root.dispatch(GpuiAction::ScmSetCommitMessage("first draft".into()), cx);
            root
        });
        cx.refresh().unwrap();
        let (first, old) = view
            .update(cx, |root, w, cx| {
                let first = root.app.gpui_scm_owner().unwrap();
                let old = root.scm_input.state().unwrap().clone();
                let tx = root.app.install_scm_pending_fixture();
                root.dispatch(GpuiAction::AddProjectPath(second), cx);
                submit(root, &old, w, cx);
                root.dispatch(GpuiAction::ScmSetCommitMessage("second draft".into()), cx);
                tx.send(Err("first refused".into())).unwrap();
                root.poll_backend(cx);
                assert_eq!(root.app.gpui_scm_draft().unwrap().2, "second draft");
                (first, old)
            })
            .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert!(!matches(root, &old));
            submit(root, &old, w, cx);
            assert!(!root.app.gpui_scm_busy());
            root.dispatch(GpuiAction::SelectProject(first.project), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            assert_eq!(root.app.gpui_scm_draft().unwrap().2, "first draft");
            assert_eq!(root.scm_input.state(), Some(&old));
            let second = root
                .scm_input
                .bindings
                .keys()
                .find(|owner| owner.project != first.project)
                .unwrap()
                .project;
            root.dispatch(GpuiAction::SelectProject(second), cx);
            root.app.install_scm_status_fixture();
            assert!(
                root.app.gpui_scm_can_submit(),
                "fixture must permit a real commit"
            );
            // Both native entities exist, but the old render still names A.
            assert_eq!(root.scm_input.state(), Some(&old));
            assert!(!matches(root, &old));
            submit(root, &old, w, cx);
            assert_eq!(root.app.gpui_scm_draft().unwrap().2, "second draft");
            assert!(!root.app.gpui_scm_busy());
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_scm_real_hook_refusal_keeps_message_then_success_clears_once(
        cx: &mut TestAppContext,
    ) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("repo");
        std::fs::create_dir(&project).unwrap();
        let init = std::process::Command::new("git")
            .args(["-c", "init.templateDir=", "init", "-q"])
            .current_dir(&project)
            .output()
            .unwrap();
        assert!(init.status.success());
        for (key, value) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.test"),
            ("commit.gpgsign", "false"),
            ("core.fsmonitor", "false"),
        ] {
            crate::git::run(&project, &["config", "--local", key, value], None).unwrap();
        }
        let hooks = project.join("fixture-hooks");
        std::fs::create_dir(&hooks).unwrap();
        crate::git::run(
            &project,
            &[
                "config",
                "--local",
                "core.hooksPath",
                hooks.to_str().unwrap(),
            ],
            None,
        )
        .unwrap();
        std::fs::write(project.join("file.txt"), "synthetic\n").unwrap();
        crate::git::stage_files(&project, &["file.txt".into()]).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            prepare(&mut root, project.clone(), cx);
            root.dispatch(
                GpuiAction::ScmSetCommitMessage("fixture\0 commit\r\n\r\nbody".into()),
                cx,
            );
            root.app.install_scm_status_fixture();
            root
        });
        cx.refresh().unwrap();
        for success in [false, true] {
            if success {
                std::fs::remove_file(&hook).unwrap();
            }
            // Finish any status refresh from the preceding job before resubmission.
            for _ in 0..400 {
                let ready = view
                    .update(cx, |root, _, cx| {
                        root.poll_backend(cx);
                        root.app.gpui_scm_can_submit()
                    })
                    .unwrap();
                if ready {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            cx.refresh().unwrap();
            view.update(cx, |root, w, cx| {
                let input = root.scm_input.state().unwrap().clone();
                submit(root, &input, w, cx);
                assert!(root.app.gpui_scm_busy());
                submit(root, &input, w, cx);
            })
            .unwrap();
            for _ in 0..400 {
                let busy = view
                    .update(cx, |root, _, cx| {
                        root.poll_backend(cx);
                        root.app.gpui_scm_busy()
                    })
                    .unwrap();
                if !busy {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            view.update(cx, |root, _, _| {
                assert!(!root.app.gpui_scm_busy());
                assert_eq!(
                    root.app.gpui_scm_draft().unwrap().2,
                    if success {
                        ""
                    } else {
                        "fixture\0 commit\r\n\r\nbody"
                    }
                );
            })
            .unwrap();
        }
        assert_eq!(
            crate::git::run(&project, &["log", "-1", "--format=%B"], None)
                .unwrap()
                .trim(),
            "fixture commit\n\nbody"
        );
        assert_eq!(
            crate::git::run(&project, &["rev-list", "--count", "HEAD"], None)
                .unwrap()
                .trim(),
            "1"
        );
    }
}
