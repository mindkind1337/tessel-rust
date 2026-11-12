//! One inline task-title editor with native selection, IME and undo.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::{
    app::gpui_bridge::GpuiAction,
    native_task_board::{Command, Scope},
};
use gpui::{App, AppContext, Context, Entity, EntityInputHandler, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
type Token = (Scope, u64, Entity<EditableTextState>);
struct Binding {
    token: Token,
    synced: String,
    too_long: bool,
    _changed: Subscription,
    _blur: Subscription,
}
#[derive(Default)]
pub(crate) struct Input {
    binding: Option<Binding>,
}
gpui::actions!(tessel_task_rename, [Save, Cancel]);
impl Input {
    pub fn focused(&self, w: &Window, cx: &App) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|b| b.token.2.read(cx).focus_handle(cx).is_focused(w))
    }
    pub fn sync(
        &mut self,
        value: Option<(Scope, u64, String)>,
        w: &mut Window,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        let Some((scope, serial, text)) = value else {
            self.binding = None;
            return;
        };
        if self.binding.as_ref().is_some_and(|b| {
            b.token.0 == scope
                && b.token.1 == serial
                && (b.token.2.read(cx).as_str() == text || (b.too_long && b.synced == text))
        }) {
            return;
        }
        let state = cx.new(|cx| EditableTextState::new(StringStorage::from(text.as_str()), cx));
        let token = (scope, serial, state.clone());
        let changed_token = token.clone();
        let changed = cx.subscribe(&state, move |root, _, _: &TextChanged, cx| {
            if matches(root, &changed_token) {
                flush(root, cx);
                cx.notify();
            }
        });
        let blur_token = token.clone();
        let focus = state.read(cx).focus_handle(cx);
        let blur = cx.on_blur(&focus, w, move |root, w, cx| {
            finish(root, &blur_token, false, w, cx)
        });
        self.binding = Some(Binding {
            token,
            synced: text,
            too_long: false,
            _changed: changed,
            _blur: blur,
        });
        state.update(cx, |s, cx| {
            s.set_selected_text_range(0..s.as_str().encode_utf16().count(), w, cx)
        });
        w.focus(&focus, cx);
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
            return div().into_any_element();
        };
        let save = b.token.clone();
        let cancel = b.token.clone();
        div()
            .flex_1()
            .min_w_0()
            .child(
                gpui_ce_elements::editable_text::text_input("native-task-rename")
                    .state(b.token.2.downgrade())
                    .key_context("EditableText TesselTaskRename")
                    .on_action(
                        cx.listener(move |root, _: &Save, w, cx| finish(root, &save, false, w, cx)),
                    )
                    .on_action(cx.listener(move |root, _: &Cancel, w, cx| {
                        finish(root, &cancel, true, w, cx)
                    }))
                    .h(px(26.))
                    .px(px(7.))
                    .border_1()
                    .border_color(tokens.accent)
                    .rounded(px(5.))
                    .text_size(px(12.5))
                    .text_color(tokens.text)
                    .bg(tokens.term),
            )
            .when(b.too_long, |el| {
                el.child(
                    div()
                        .text_size(px(10.))
                        .text_color(tokens.warn)
                        .child(if french {
                            "Titre limité à 1 000 caractères"
                        } else {
                            "Title limit: 1,000 characters"
                        }),
                )
            })
            .into_any_element()
    }
}
fn matches(root: &TesselGpuiRoot, token: &Token) -> bool {
    root.task_rename_input.binding.as_ref().is_some_and(|b| {
        b.token == *token
            && root.app.gpui_task_rename_input() == Some((token.0, token.1, b.synced.as_str()))
    })
}
pub(crate) fn flush(root: &mut TesselGpuiRoot, cx: &mut Context<TesselGpuiRoot>) {
    let Some(b) = &root.task_rename_input.binding else {
        return;
    };
    let token = b.token.clone();
    if !matches(root, &token) {
        return;
    }
    let base = b.synced.clone();
    let text = token.2.read(cx).as_str().to_owned();
    let too_long = text.chars().count() > crate::tasks_board_panel::MAX_TASK_TITLE_CHARS;
    if !too_long && text != base {
        root.app.apply_gpui_action(GpuiAction::NativeTaskBoard {
            scope: token.0,
            command: Command::Edit {
                serial: token.1,
                base,
                text,
            },
        });
        if let Some((_, _, text)) = root.app.gpui_task_rename_input() {
            root.task_rename_input.binding.as_mut().unwrap().synced = text.to_owned();
        }
    }
    root.task_rename_input.binding.as_mut().unwrap().too_long = too_long;
}
fn finish(
    root: &mut TesselGpuiRoot,
    token: &Token,
    cancel: bool,
    w: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    if !matches(root, token)
        || token
            .2
            .update(cx, |s, cx| s.marked_text_range(w, cx).is_some())
    {
        return;
    }
    flush(root, cx);
    if !cancel
        && root
            .task_rename_input
            .binding
            .as_ref()
            .is_some_and(|b| b.too_long)
    {
        return;
    }
    let command = if cancel {
        Command::CancelEdit { serial: token.1 }
    } else {
        Command::Save {
            serial: token.1,
            base: token.2.read(cx).as_str().to_owned(),
        }
    };
    root.app.apply_gpui_action(GpuiAction::NativeTaskBoard {
        scope: token.0,
        command,
    });
    root.focus_input = FocusInputTarget::None;
    if token.2.read(cx).focus_handle(cx).is_focused(w) {
        let card = root
            .task_card_input
            .focused(w, cx)
            .and_then(|id| root.task_card_input.handle(id))
            .cloned();
        w.focus(card.as_ref().unwrap_or(&root.focus_handle), cx);
    }
    cx.notify();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{tasks_board_panel::TasksBoardAction, workspace::SideTab};
    use gpui::TestAppContext;
    #[gpui::test]
    fn native_task_rename_supports_unicode_undo_cancel_and_stale_card(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir(&project).unwrap();
        let view = cx.add_window(|w, cx| {
            w.activate();
            let mut root = TesselGpuiRoot::new_in_dir(Some(temp.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(project), cx);
            root.dispatch(GpuiAction::SelectSideTab(SideTab::Tasks), cx);
            root.dispatch(
                GpuiAction::TasksBoard(TasksBoardAction::Add("Original".into())),
                cx,
            );
            let snap = root.app.gpui_snapshot(960., 640., 4.);
            let scope = snap.side_panels.task_board.scope.unwrap();
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope,
                    command: Command::BeginRename(snap.side_panels.tasks[0].id),
                },
                cx,
            );
            root
        });
        // Platform activation is queued on the test executor, unlike a focus handle.
        cx.run_until_parked();
        cx.refresh().unwrap();
        let token = view
            .update(cx, |root, w, cx| {
                let token = root
                    .task_rename_input
                    .binding
                    .as_ref()
                    .unwrap()
                    .token
                    .clone();
                assert!(w.is_window_active(), "blur needs an active test window");
                assert!(root.task_rename_input.focused(w, cx));
                token.2.update(cx, |s, cx| {
                    s.push_undo_stop();
                    s.replace_text_in_range(None, "Québec 😀", w, cx);
                    s.push_undo_stop();
                });
                token
            })
            .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            assert_eq!(token.2.read(cx).as_str(), "Original");
            token.2.update(cx, |s, cx| {
                s.replace_text_in_range(Some(0..8), "Québec 😀", w, cx)
            });
            flush(root, cx);
            finish(root, &token, false, w, cx);
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].title,
                "Québec 😀"
            );
            finish(root, &token, false, w, cx); // Old entity cannot submit twice.
            let snap = root.app.gpui_snapshot(960., 640., 4.);
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope: token.0,
                    command: Command::BeginRename(snap.side_panels.tasks[0].id),
                },
                cx,
            );
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            let next = root
                .task_rename_input
                .binding
                .as_ref()
                .unwrap()
                .token
                .clone();
            assert_ne!(next.1, token.1);
            next.2.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "日", None, w, cx)
            });
            finish(root, &next, false, w, cx);
            assert!(root.app.gpui_task_rename_input().is_some());
            next.2.update(cx, |s, cx| {
                s.unmark_text(w, cx);
                let len = s.as_str().encode_utf16().count();
                s.replace_text_in_range(Some(0..len), &"x".repeat(1001), w, cx);
            });
            finish(root, &next, false, w, cx);
            assert!(root.app.gpui_task_rename_input().is_some());
            next.2.update(cx, |s, cx| {
                let len = s.as_str().encode_utf16().count();
                s.replace_text_in_range(Some(0..len), "Discarded", w, cx);
            });
            finish(root, &next, true, w, cx);
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].title,
                "Québec 😀"
            );
            let id = root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].id;
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope: token.0,
                    command: Command::BeginRename(id),
                },
                cx,
            );
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            let next = root
                .task_rename_input
                .binding
                .as_ref()
                .unwrap()
                .token
                .clone();
            let id = root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].id;
            root.dispatch(
                GpuiAction::TasksBoard(TasksBoardAction::Move { id, column: 1 }),
                cx,
            );
            finish(root, &next, false, w, cx);
            assert!(root.app.gpui_task_rename_input().is_none());
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].title,
                "Québec 😀"
            );
        })
        .unwrap();
        view.update(cx, |root, _, cx| {
            let id = root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].id;
            root.dispatch(
                GpuiAction::TasksBoard(TasksBoardAction::Move { id, column: 0 }),
                cx,
            );
            root.dispatch(
                GpuiAction::NativeTaskBoard {
                    scope: token.0,
                    command: Command::BeginRename(id),
                },
                cx,
            );
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, w, cx| {
            let input = root
                .task_rename_input
                .binding
                .as_ref()
                .unwrap()
                .token
                .2
                .clone();
            assert!(w.is_window_active(), "blur needs an active test window");
            assert!(root.task_rename_input.focused(w, cx));
            input.update(cx, |s, cx| {
                s.replace_text_in_range(None, "Saved on blur", w, cx)
            });
            flush(root, cx);
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, w, cx| {
            w.focus(&root.focus_handle, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_snapshot(960., 640., 4.).side_panels.tasks[0].title,
                "Saved on blur"
            );
            assert!(root.app.gpui_task_rename_input().is_none());
        })
        .unwrap();
    }
}
