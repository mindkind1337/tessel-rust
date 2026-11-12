//! Native editor tools keep edits scoped to their document instance.
use super::{FocusInputTarget, TesselGpuiRoot};
use crate::app::gpui_bridge::{EditorToolField, GpuiAction, GpuiActiveDocumentSummary};
use gpui::{App, AppContext, Context, Entity, EntityInputHandler, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;

pub(crate) fn target(field: EditorToolField) -> FocusInputTarget {
    match field {
        EditorToolField::Find => FocusInputTarget::EditorFind,
        EditorToolField::Replace => FocusInputTarget::EditorReplace,
        EditorToolField::Goto => FocusInputTarget::EditorGoto,
    }
}
fn field(target: FocusInputTarget) -> Option<EditorToolField> {
    match target {
        FocusInputTarget::EditorFind => Some(EditorToolField::Find),
        FocusInputTarget::EditorReplace => Some(EditorToolField::Replace),
        FocusInputTarget::EditorGoto => Some(EditorToolField::Goto),
        _ => None,
    }
}
struct Binding {
    state: Entity<EditableTextState>,
    synced: String,
    too_long: bool,
    _changed: Subscription,
}
#[derive(Default)]
pub(crate) struct Inputs {
    pub select_on_focus: Option<(u64, EditorToolField)>,
    bindings: HashMap<(u64, EditorToolField), Binding>,
    replacement_too_large: Option<u64>,
}
gpui::actions!(tessel_editor_tools, [Submit, Previous, Dismiss]);
#[derive(Clone, Copy)]
pub(crate) enum Command {
    Find(isize),
    Replace(bool),
    Goto,
    Close(EditorToolField),
}
impl Inputs {
    pub fn focused(&self, window: &Window, cx: &App) -> Option<FocusInputTarget> {
        self.bindings.iter().find_map(|((_, f), b)| {
            b.state
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
                .then_some(target(*f))
        })
    }
    pub fn get(&self, id: u64, f: EditorToolField) -> Option<&Entity<EditableTextState>> {
        self.bindings.get(&(id, f)).map(|b| &b.state)
    }
    pub fn for_target(&self, id: u64, t: FocusInputTarget) -> Option<&Entity<EditableTextState>> {
        self.get(id, field(t)?)
    }
    pub fn sync(
        &mut self,
        doc: Option<&GpuiActiveDocumentSummary>,
        open: &[u64],
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        self.bindings.retain(|(id, f), _| {
            open.contains(id)
                && doc.is_none_or(|d| {
                    d.instance_id != *id
                        || match f {
                            EditorToolField::Find => d.find_open,
                            EditorToolField::Replace => d.find_open && !d.media && !d.preview_mode,
                            EditorToolField::Goto => d.goto_open,
                        }
                })
        });
        let Some(doc) = doc else { return };
        for (f, value, visible) in [
            (EditorToolField::Find, &doc.find_query, doc.find_open),
            (
                EditorToolField::Replace,
                &doc.replace_text,
                doc.find_open && !doc.media && !doc.preview_mode,
            ),
            (EditorToolField::Goto, &doc.goto_line, doc.goto_open),
        ] {
            if !visible {
                continue;
            }
            let id = doc.instance_id;
            let key = (id, f);
            if self.bindings.get(&key).is_some_and(|b| {
                b.state.read(cx).as_str() == value || (b.too_long && b.synced == *value)
            }) {
                continue;
            }
            let state =
                cx.new(|cx| EditableTextState::new(StringStorage::from(value.as_str()), cx));
            let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
                let Some(b) = root.editor_input.tools.bindings.get(&key) else {
                    return;
                };
                if b.state != state || root.app.gpui_editor_tool_value(id, f).is_none() {
                    return;
                }
                let base = b.synced.clone();
                let too_long = state.read(cx).as_str().len() > crate::files::MAX_TEXT;
                root.editor_input
                    .tools
                    .bindings
                    .get_mut(&key)
                    .unwrap()
                    .too_long = too_long;
                root.editor_input.tools.replacement_too_large = None;
                if too_long {
                    cx.notify();
                    return;
                }
                let text = state.read(cx).as_str().to_owned();
                if base != text {
                    root.dispatch(
                        GpuiAction::ApplyNativeEditorToolEdit {
                            instance_id: id,
                            field: f,
                            base,
                            text,
                        },
                        cx,
                    );
                    acknowledge(root, id, f);
                }
            });
            self.bindings.insert(
                key,
                Binding {
                    state,
                    synced: value.clone(),
                    too_long: false,
                    _changed: changed,
                },
            );
        }
    }
    pub fn render(
        &self,
        doc: &GpuiActiveDocumentSummary,
        f: EditorToolField,
        french: bool,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> gpui::AnyElement {
        use gpui::{
            InteractiveElement, IntoElement, ParentElement, Styled, div, prelude::FluentBuilder, px,
        };
        let id = doc.instance_id;
        let Some(b) = self.bindings.get(&(id, f)) else {
            return div().into_any_element();
        };
        let (name, width, placeholder, submit) = match f {
            EditorToolField::Find => (
                "native-editor-find",
                200.,
                if french { "Rechercher…" } else { "Find…" },
                Command::Find(1),
            ),
            EditorToolField::Replace => (
                "native-editor-replace",
                160.,
                if french {
                    "Remplacer par…"
                } else {
                    "Replace with…"
                },
                Command::Replace(false),
            ),
            EditorToolField::Goto => ("native-editor-goto", 110., "line:col", Command::Goto),
        };
        let submit_state = b.state.clone();
        let previous_state = b.state.clone();
        let dismiss_state = b.state.clone();
        let input = gpui_ce_elements::editable_text::text_input(name)
            .state(b.state.downgrade())
            .placeholder(placeholder)
            .key_context("EditableText TesselEditorTools")
            .on_action(cx.listener(move |root, _: &Submit, w, cx| {
                if root.editor_input.tools.get(id, f) == Some(&submit_state) {
                    run(root, id, submit, w, cx);
                }
            }))
            .on_action(cx.listener(move |root, _: &Previous, w, cx| {
                if root.editor_input.tools.get(id, f) != Some(&previous_state) {
                    return;
                }
                run(
                    root,
                    id,
                    if f == EditorToolField::Find {
                        Command::Find(-1)
                    } else {
                        submit
                    },
                    w,
                    cx,
                )
            }))
            .on_action(cx.listener(move |root, _: &Dismiss, w, cx| {
                if root.editor_input.tools.get(id, f) == Some(&dismiss_state) {
                    run(root, id, Command::Close(f), w, cx);
                }
            }))
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(move |root, _, _, _| root.focus_input = target(f)),
            )
            .w_full()
            .h(px(25.))
            .px(px(7.))
            .py(px(3.))
            .text_size(px(11.5))
            .text_color(tokens.text_strong);
        div()
            .w(px(width))
            .flex()
            .flex_col()
            .bg(tokens.term)
            .rounded(px(4.))
            .border_1()
            .border_color(tokens.border_strong)
            .child(input)
            .when(
                b.too_long
                    || (f == EditorToolField::Replace && self.replacement_too_large == Some(id)),
                |el| {
                    el.child(
                        div()
                            .text_size(px(10.))
                            .text_color(tokens.warn)
                            .child(if french {
                                "Limite de 2 Mio dépassée"
                            } else {
                                "2 MiB limit exceeded"
                            }),
                    )
                },
            )
            .into_any_element()
    }
}
fn acknowledge(root: &mut TesselGpuiRoot, id: u64, f: EditorToolField) {
    if let Some(value) = root.app.gpui_editor_tool_value(id, f)
        && let Some(b) = root.editor_input.tools.bindings.get_mut(&(id, f))
    {
        b.synced = value.to_owned();
    }
}
pub(crate) fn open(
    root: &mut TesselGpuiRoot,
    f: EditorToolField,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let Some(id) = root.app.gpui_editor_tool_document_id() else {
        return;
    };
    if f == EditorToolField::Replace && root.app.gpui_editor_document().is_none_or(|doc| doc.media)
    {
        return;
    }

    if let Some(state) = root.editor_input.tools.get(id, f).cloned()
        && state.update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
    {
        return;
    }
    if root.app.gpui_editor_tool_value(id, f).is_none() {
        root.dispatch(
            match f {
                EditorToolField::Find => GpuiAction::ToggleEditorFind,
                EditorToolField::Replace => GpuiAction::ToggleEditorReplace,
                EditorToolField::Goto => GpuiAction::ToggleEditorGoto,
            },
            cx,
        );
    }
    if root.app.gpui_editor_tool_value(id, f).is_some() {
        root.focus_input = target(f);
        root.editor_input.tools.select_on_focus = Some((id, f));
        cx.notify();
    }
}

pub(crate) fn run(
    root: &mut TesselGpuiRoot,
    id: u64,
    command: Command,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let fields: &[EditorToolField] = match command {
        Command::Goto | Command::Close(EditorToolField::Goto) => &[EditorToolField::Goto],
        Command::Replace(_) => &[EditorToolField::Find, EditorToolField::Replace],
        Command::Close(EditorToolField::Replace) => &[EditorToolField::Replace],
        _ => &[EditorToolField::Find],
    };
    let mut edits = Vec::new();
    for &f in fields {
        let Some(base) = root.app.gpui_editor_tool_value(id, f) else {
            return;
        };
        let Some(b) = root.editor_input.tools.bindings.get(&(id, f)) else {
            return;
        };
        if base != b.synced {
            return;
        }
        let state = b.state.clone();
        if state.update(cx, |s, cx| s.marked_text_range(window, cx).is_some()) {
            return;
        }
        if !matches!(command, Command::Close(_))
            && state.read(cx).as_str().len() > crate::files::MAX_TEXT
        {
            return;
        }
        edits.push((f, base.to_owned(), state.read(cx).as_str().to_owned()));
    }
    if let Command::Close(f) = command {
        root.dispatch(
            if f == EditorToolField::Goto {
                GpuiAction::ToggleEditorGoto
            } else {
                GpuiAction::ToggleEditorFind
            },
            cx,
        );
        root.focus_input = FocusInputTarget::EditorBody;
        return;
    }
    for (f, base, text) in edits {
        root.dispatch(
            GpuiAction::ApplyNativeEditorToolEdit {
                instance_id: id,
                field: f,
                base,
                text: text.clone(),
            },
            cx,
        );
        acknowledge(root, id, f);
        if root.app.gpui_editor_tool_value(id, f) != Some(text.as_str()) {
            return;
        }
    }
    match command {
        Command::Find(delta) => root.dispatch(GpuiAction::StepEditorFind(delta), cx),
        Command::Goto => {
            root.dispatch(GpuiAction::ApplyEditorGoto, cx);
            if root
                .app
                .gpui_editor_tool_value(id, EditorToolField::Goto)
                .is_none()
            {
                root.focus_input = FocusInputTarget::EditorBody;
            }
        }
        Command::Replace(all) => {
            let Some(doc) = root.app.gpui_editor_document() else {
                return;
            };
            if doc.instance_id() != id || doc.media {
                return;
            }
            let Some(state) = root.editor_input.get(id).cloned() else {
                return;
            };
            if state.read(cx).as_str() != doc.text {
                return;
            }
            let plan = plan_replacement(
                &doc.text,
                &doc.tools.find_query,
                &doc.tools.replace_text,
                doc.tools.match_index,
                all,
            );
            match plan {
                Ok(Some((range, replacement))) => state.update(cx, |s, cx| {
                    if s.marked_text_range(window, cx).is_some() {
                        return;
                    }
                    let range = s.as_str()[..range.start].encode_utf16().count()
                        ..s.as_str()[..range.end].encode_utf16().count();
                    s.replace_text_in_range(Some(range), &replacement, window, cx);
                }),
                Err(()) => {
                    root.editor_input.tools.replacement_too_large = Some(id);
                    cx.notify();
                }
                Ok(None) => {}
            }
        }
        Command::Close(_) => {}
    }
}
fn plan_replacement(
    source: &str,
    query: &str,
    replacement: &str,
    index: usize,
    all: bool,
) -> Result<Option<(std::ops::Range<usize>, String)>, ()> {
    let hits = crate::files::find_char_matches(source, query);
    if hits.is_empty() {
        return Ok(None);
    }
    let hits = if all {
        hits.as_slice()
    } else {
        std::slice::from_ref(&hits[index.min(hits.len() - 1)])
    };
    let offsets: Vec<usize> = source
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(source.len()))
        .collect();
    let removed: usize = hits.iter().map(|(a, b, _)| offsets[*b] - offsets[*a]).sum();
    let final_len = source
        .len()
        .checked_sub(removed)
        .and_then(|n| {
            replacement
                .len()
                .checked_mul(hits.len())
                .and_then(|added| n.checked_add(added))
        })
        .ok_or(())?;
    if final_len > crate::files::MAX_TEXT {
        return Err(());
    }
    let start = offsets[hits[0].0];
    let end = offsets[hits.last().unwrap().1];
    let mut next = String::with_capacity(final_len - (source.len() - (end - start)));
    let mut cursor = start;
    for &(a, b, _) in hits {
        let a = offsets[a];
        let b = offsets[b];
        next.push_str(&source[cursor..a]);
        next.push_str(replacement);
        cursor = b;
    }
    next.push_str(&source[cursor..end]);
    Ok(Some((start..end, next)))
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use crate::files::Document;
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
    fn native_editor_tools_flush_unicode_replace_and_source_undo(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edit.txt");
        let source = "é😀 apple\nAPPLE apple";
        std::fs::write(&path, source).unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-f");
        cx.refresh().unwrap();
        let find = view
            .update(cx, |root, window, cx| {
                let input = root
                    .editor_input
                    .tools
                    .get(id, EditorToolField::Find)
                    .unwrap()
                    .clone();
                assert!(input.read(cx).focus_handle(cx).is_focused(window));
                input.update(cx, |s, cx| {
                    s.replace_text_in_range(None, "app😀e", window, cx);
                    s.set_selected_text_range(3..5, window, cx);
                    s.paste(gpui::ClipboardItem::new_string("l".into()), window, cx);
                });
                input
            })
            .unwrap();
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-f");
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            let state = root
                .editor_input
                .tools
                .get(id, EditorToolField::Find)
                .unwrap()
                .clone();
            assert_eq!(state, find);
            assert_eq!(
                state
                    .update(cx, |s, cx| s
                        .selected_text_range(false, window, cx)
                        .unwrap())
                    .range,
                0..5
            );
        })
        .unwrap();
        view.update(cx, |root, window, cx| {
            assert_eq!(
                root.app.gpui_editor_tool_value(id, EditorToolField::Find),
                Some("apple")
            );
            let replace = root
                .editor_input
                .tools
                .get(id, EditorToolField::Replace)
                .unwrap()
                .clone();
            replace.update(cx, |s, cx| {
                s.replace_and_mark_text_in_range(None, "🍎", None, window, cx)
            });
            run(
                root,
                id,
                Command::Close(EditorToolField::Replace),
                window,
                cx,
            );
            assert!(
                root.app
                    .gpui_editor_tool_value(id, EditorToolField::Replace)
                    .is_some()
            );
            replace.update(cx, |s, cx| s.replace_text_in_range(None, "🍎", window, cx));
            // Submit before TextChanged is delivered: both native values must be flushed.
            run(root, id, Command::Replace(true), window, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_editor_document().unwrap().text,
                "é😀 🍎\n🍎 🍎"
            )
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "escape");
        cx.refresh().unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_editor_document().unwrap().text, source)
        })
        .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
        cx.run_until_parked();
        view.update(cx, |root, window, cx| {
            assert_eq!(
                root.app.gpui_editor_document().unwrap().text,
                "é😀 🍎\n🍎 🍎"
            );
            find.update(cx, |s, cx| {
                s.replace_text_in_range(None, "stale", window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(view.into(), "ctrl-f");
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_editor_tool_value(id, EditorToolField::Find),
                Some("apple")
            )
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_editor_tools_ime_goto_and_stale_document_actions(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "first\n😀last").unwrap();
        std::fs::write(&b, "second").unwrap();
        let da = Document::open(dir.path(), &a).unwrap();
        let id = da.instance_id();
        let db = Document::open(dir.path(), &b).unwrap();
        let other = db.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(vec![da, db], 0);
            root
        });
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-g");
        cx.refresh().unwrap();
        let input = view
            .update(cx, |root, window, cx| {
                let input = root
                    .editor_input
                    .tools
                    .get(id, EditorToolField::Goto)
                    .unwrap()
                    .clone();
                input.update(cx, |s, cx| {
                    s.replace_and_mark_text_in_range(None, "2:2", None, window, cx)
                });
                input
            })
            .unwrap();
        cx.simulate_keystrokes(view.into(), "enter");
        view.update(cx, |root, window, cx| {
            assert!(
                root.app
                    .gpui_editor_tool_value(id, EditorToolField::Goto)
                    .is_some()
            );
            input.update(cx, |s, cx| s.replace_text_in_range(None, "2:2", window, cx));
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "enter");
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(
                root.app
                    .gpui_editor_tool_value(id, EditorToolField::Goto)
                    .is_none()
            );
            assert!(root.editor_input.focused(window, cx));
            let state = root.editor_input.get(id).unwrap().clone();
            let selection = state.update(cx, |s, cx| {
                s.selected_text_range(false, window, cx).unwrap()
            });
            assert_eq!(selection.range, 8..8); // first\n + UTF-16 emoji
            root.dispatch(GpuiAction::SelectDocumentTab(1), cx);
            root.dispatch(GpuiAction::ToggleEditorGoto, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            input.update(cx, |s, cx| s.replace_text_in_range(None, "1:2", window, cx));
            run(root, id, Command::Goto, window, cx);
            assert_eq!(
                root.app
                    .gpui_editor_tool_value(other, EditorToolField::Goto),
                Some("")
            );
            root.dispatch(GpuiAction::OpenHelp, cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.editor_input.tools.focused(window, cx).is_none())
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_editor_replace_shortcut_refuses_readonly_preview(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let doc = Document::from_diff(
            dir.path(),
            dir.path().join("diff.txt"),
            "diff".into(),
            "readonly".into(),
            false,
        );
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            open(root, EditorToolField::Replace, window, cx);
            assert!(
                root.app
                    .gpui_editor_tool_value(id, EditorToolField::Find)
                    .is_none()
            );
            run(root, id, Command::Replace(true), window, cx);
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "readonly");
        })
        .unwrap();
    }
    #[test]
    fn replacement_plan_bounds_unicode_and_single_hit() {
        assert_eq!(
            plan_replacement("😀 a A", "a", "é", 0, true).unwrap(),
            Some((5..8, "é é".into()))
        );
        assert_eq!(
            plan_replacement("a a", "a", "x", 1, false).unwrap(),
            Some((2..3, "x".into()))
        );
        assert!(
            plan_replacement("aaa", "a", &"x".repeat(crate::files::MAX_TEXT), 0, true).is_err()
        );
        assert_eq!(plan_replacement("abc", "", "x", 0, true).unwrap(), None);
    }
}
