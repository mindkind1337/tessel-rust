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
                    el.child(div().text_size(px(10.)).text_color(tokens.warn).child(
                        match (b.too_long, french) {
                            (true, true) => "Limite du champ de 2 Mio dépassée",
                            (true, false) => "Field exceeds 2 MiB",
                            (false, true) => "Le document dépasserait 50 Mio",
                            (false, false) => "Document would exceed 50 MiB",
                        },
                    ))
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
        if !matches!(command, Command::Close(_)) {
            edits.push((f, base.to_owned(), state.read(cx).as_str().to_owned()));
        }
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
        Command::Replace(all) => replace(root, id, all, window, cx),
        Command::Close(_) => {}
    }
}
fn apply_pending_selection(root: &mut TesselGpuiRoot, id: u64, cx: &mut Context<TesselGpuiRoot>) {
    if let Some((start, end)) = root.app.gpui_take_editor_selection(id)
        && let Some(state) = root.editor_input.get(id).cloned()
    {
        state.update(cx, |s, cx| {
            s.move_to(start, cx);
            s.select_to(end, cx);
        });
    }
}
fn select_match(
    root: &mut TesselGpuiRoot,
    id: u64,
    from_char: usize,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let Some(doc) = root.app.gpui_editor_document() else {
        return;
    };
    if doc.instance_id() != id {
        return;
    }
    root.dispatch(
        GpuiAction::SelectNativeEditorMatch {
            instance_id: id,
            base: doc.text.clone(),
            query: doc.tools.find_query.clone(),
            from_char,
        },
        cx,
    );
    apply_pending_selection(root, id, cx);
}
fn utf16_to_char(source: &str, offset: usize) -> usize {
    let mut units = 0;
    for (index, ch) in source.chars().enumerate() {
        if units >= offset {
            return index;
        }
        units += ch.len_utf16();
    }
    source.chars().count()
}
fn replace(
    root: &mut TesselGpuiRoot,
    id: u64,
    all: bool,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    let Some(state) = root.editor_input.get(id).cloned() else {
        return;
    };
    if state.update(cx, |s, cx| s.marked_text_range(window, cx).is_some())
        || !super::editor_input::flush(root, id, cx)
    {
        return;
    }
    apply_pending_selection(root, id, cx);
    let Some(doc) = root.app.gpui_editor_document() else {
        return;
    };
    if doc.instance_id() != id || doc.media {
        return;
    }
    let hits = crate::files::find_char_matches(&doc.text, &doc.tools.find_query);
    if hits.is_empty() {
        return;
    }
    let mut index = doc.tools.match_index;
    if !all {
        let Some(selection) = state.update(cx, |s, cx| s.selected_text_range(false, window, cx))
        else {
            return;
        };
        let start = utf16_to_char(&doc.text, selection.range.start);
        let end = utf16_to_char(&doc.text, selection.range.end);
        index = hits.iter().position(|h| h.0 >= start).unwrap_or(0);
        if hits[index].0 != start || hits[index].1 != end {
            select_match(root, id, start, cx);
            return;
        }
    }
    let plan =
        crate::files::replacement_edit(&doc.text, &hits, &doc.tools.replace_text, index, all);
    root.editor_input.tools.replacement_too_large = None;
    match plan {
        Ok(Some((range, replacement))) => {
            let from = state.read(cx).as_str()[..range.start].chars().count()
                + replacement.chars().count();
            state.update(cx, |s, cx| {
                let range = s.as_str()[..range.start].encode_utf16().count()
                    ..s.as_str()[..range.end].encode_utf16().count();
                s.push_undo_stop();
                s.replace_text_in_range(Some(range), &replacement, window, cx);
                s.push_undo_stop();
            });
            if super::editor_input::flush(root, id, cx) && !all {
                select_match(root, id, from, cx);
            }
        }
        Err(_) => {
            root.editor_input.tools.replacement_too_large = Some(id);
            cx.notify();
        }
        Ok(None) => {}
    }
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
            assert_eq!(selection.range, 6..6); // Column two is halfway across the emoji.
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
    fn native_single_replace_selects_then_advances_and_keeps_separate_undo(
        cx: &mut TestAppContext,
    ) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("single.txt");
        std::fs::write(&path, "aa").unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root.app.apply_gpui_action(GpuiAction::ToggleEditorFind);
            root.app
                .apply_gpui_action(GpuiAction::SetEditorFindQuery("a".into()));
            root.app
                .apply_gpui_action(GpuiAction::SetEditorReplaceText("b".into()));
            root.focus_input = FocusInputTarget::EditorReplace;
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            let state = root.editor_input.get(id).unwrap().clone();
            state.update(cx, |s, cx| s.move_to(0, cx));
            run(root, id, Command::Replace(false), window, cx);
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "aa");
            assert_eq!(
                state
                    .update(cx, |s, cx| s
                        .selected_text_range(false, window, cx)
                        .unwrap())
                    .range,
                0..1
            );
            run(root, id, Command::Replace(false), window, cx);
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "ba");
            assert_eq!(
                state
                    .update(cx, |s, cx| s
                        .selected_text_range(false, window, cx)
                        .unwrap())
                    .range,
                1..2
            );
            // No repaint or queued event between clicks: native text and model must stay aligned.
            run(root, id, Command::Replace(false), window, cx);
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "bb");
            run(root, id, Command::Close(EditorToolField::Find), window, cx);
            state.update(cx, |s, cx| {
                s.replace_text_in_range(Some(2..2), "c", window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.refresh().unwrap();
        for expected in ["bb", "ba", "aa", "aa"] {
            cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
            cx.run_until_parked();
            view.update(cx, |root, _, _| {
                assert_eq!(root.app.gpui_editor_document().unwrap().text, expected)
            })
            .unwrap();
        }
        for expected in ["ba", "bb", "bbc"] {
            cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
            cx.run_until_parked();
            view.update(cx, |root, _, _| {
                assert_eq!(root.app.gpui_editor_document().unwrap().text, expected)
            })
            .unwrap();
        }
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        cx.simulate_keystrokes(view.into(), "d");
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "bbd")
        })
        .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "bb")
        })
        .unwrap();
    }
    #[gpui::test]
    fn native_single_replace_skips_inserted_matches_and_wraps_unicode(cx: &mut TestAppContext) {
        bindings(cx);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unicode.txt");
        std::fs::write(&path, "😀a 😀a").unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root.app.apply_gpui_action(GpuiAction::ToggleEditorFind);
            root.app
                .apply_gpui_action(GpuiAction::SetEditorFindQuery("😀a".into()));
            root.app
                .apply_gpui_action(GpuiAction::SetEditorReplaceText("😀aX".into()));
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            let state = root.editor_input.get(id).unwrap().clone();
            run(root, id, Command::Replace(false), window, cx);
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "😀aX 😀a");
            assert_eq!(
                state
                    .update(cx, |s, cx| s
                        .selected_text_range(false, window, cx)
                        .unwrap())
                    .range,
                5..8
            );
            assert_eq!(
                root.app.gpui_editor_document().unwrap().tools.match_index,
                1
            );
            run(root, id, Command::Replace(false), window, cx);
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "😀aX 😀aX");
            assert_eq!(
                state
                    .update(cx, |s, cx| s
                        .selected_text_range(false, window, cx)
                        .unwrap())
                    .range,
                0..3
            );
            assert_eq!(
                root.app.gpui_editor_document().unwrap().tools.match_index,
                0
            );
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
        let plan_replacement = |source: &str, query: &str, replacement: &str, index, all| {
            crate::files::replacement_edit(
                source,
                &crate::files::find_char_matches(source, query),
                replacement,
                index,
                all,
            )
        };
        assert_eq!(
            plan_replacement("aaaaa", "aa", "x", 0, true).unwrap(),
            Some((0..4, "xx".into()))
        );
        assert_eq!(
            plan_replacement("😀İİİ", "i\u{307}i\u{307}", "X", 0, true).unwrap(),
            Some((4..8, "X".into()))
        );

        assert_eq!(
            plan_replacement("😀 a A", "a", "é", 0, true).unwrap(),
            Some((5..8, "é é".into()))
        );
        assert_eq!(
            plan_replacement("a a", "a", "x", 1, false).unwrap(),
            Some((2..3, "x".into()))
        );
        assert!(
            plan_replacement(
                "aaa",
                "a",
                &"x".repeat(crate::files::MAX_LOCAL_EDIT_BYTES / 3 + 1),
                0,
                true
            )
            .is_err()
        );
        assert_eq!(plan_replacement("abc", "", "x", 0, true).unwrap(), None);
    }
}
