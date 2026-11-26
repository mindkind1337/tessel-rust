//! Native editor entities are retained per open document, never per filename or tab index.
use super::{TesselGpuiRoot, editor_indent, editor_syntax};
use crate::app::gpui_bridge::{GpuiAction, GpuiActiveDocumentSummary};
use gpui::{App, AppContext, Context, Entity, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;

struct Binding {
    state: Entity<EditableTextState>,
    synced: String,
    _changed: Subscription,
    _caret: Subscription,
    indentation: editor_indent::Options,
}

gpui::actions!(tessel_local_editor, [Indent, Outdent]);

pub(crate) fn flush(root: &mut TesselGpuiRoot, id: u64, cx: &mut Context<TesselGpuiRoot>) -> bool {
    let Some(doc) = root.app.gpui_editor_document() else {
        return false;
    };
    if doc.instance_id() != id || doc.media {
        return false;
    }
    let Some(binding) = root.editor_input.bindings.get(&id) else {
        return false;
    };
    if binding.synced != doc.text {
        return false;
    }
    let text = binding.state.read(cx).as_str().to_owned();
    let base = binding.synced.clone();
    if text != base {
        root.dispatch(
            GpuiAction::ApplyNativeDocumentEdit {
                instance_id: id,
                base,
                text: text.clone(),
            },
            cx,
        );
    }
    if root
        .app
        .gpui_editor_document()
        .is_some_and(|d| d.instance_id() == id && d.text == text)
    {
        if let Some(b) = root.editor_input.bindings.get_mut(&id) {
            b.synced = text;
        }
        return true;
    }
    false
}

fn apply_indent(
    root: &mut TesselGpuiRoot,
    outdent: bool,
    window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) {
    use gpui::EntityInputHandler;
    if root.effective_focus_target() != super::FocusInputTarget::EditorBody {
        return;
    }
    let Some(doc) = root.app.gpui_editor_document() else {
        return;
    };
    if doc.media {
        return;
    }
    let Some(binding) = root.editor_input.bindings.get(&doc.instance_id()) else {
        return;
    };
    let state = binding.state.clone();
    if state.read(cx).as_str() != doc.text {
        return;
    }
    let options = binding.indentation;
    state.update(cx, |state, cx| {
        if state.marked_text_range(window, cx).is_some() {
            return;
        }
        let Some(selected) = state.selected_text_range(false, window, cx) else {
            return;
        };
        let source = state.as_str();
        let to_byte = |offset: usize| {
            let mut count = 0;
            for (byte, ch) in source.char_indices() {
                if count >= offset {
                    return byte;
                }
                count += ch.len_utf16();
            }
            source.len()
        };
        let Some(edit) = editor_indent::plan(
            source,
            to_byte(selected.range.start)..to_byte(selected.range.end),
            outdent,
            options,
        ) else {
            return;
        };
        if source.len() - edit.range.len() + edit.replacement.len() > crate::files::MAX_TEXT {
            return;
        }
        let range = source[..edit.range.start].encode_utf16().count()
            ..source[..edit.range.end].encode_utf16().count();
        state.replace_text_in_range(Some(range), &edit.replacement, window, cx);
        let source = state.as_str();
        let start = source[..edit.selection.start].encode_utf16().count();
        let end = source[..edit.selection.end].encode_utf16().count();
        state.set_selected_text_range(
            if selected.reversed {
                end..start
            } else {
                start..end
            },
            window,
            cx,
        );
    });
}

#[cfg(all(test, feature = "visual-test"))]
mod tests {
    use super::*;
    use crate::files::Document;
    use gpui::{EntityInputHandler, TestAppContext};

    #[gpui::test]
    fn native_indent_shortcuts_preserve_selection_and_undo(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("indent.txt");
        std::fs::write(&path, "é😀\r\nz\r\n").unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            root.editor_input.get(id).unwrap().update(cx, |state, cx| {
                // Anchor after cursor: this is a backward selection, not an iterator.
                state.set_selected_text_range(std::ops::Range { start: 8, end: 0 }, window, cx)
            })
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "tab");
        view.update(cx, |root, window, cx| {
            assert_eq!(
                root.app.gpui_editor_document().unwrap().text,
                "    é😀\r\n    z\r\n"
            );
            root.editor_input.get(id).unwrap().update(cx, |state, cx| {
                let selection = state.selected_text_range(false, window, cx).unwrap();
                assert_eq!(selection.range, 0..16);
                assert!(selection.reversed);
            });
        })
        .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_editor_document().unwrap().text,
                "é😀\r\nz\r\n"
            )
        })
        .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "shift-tab");
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_editor_document().unwrap().text,
                "é😀\r\nz\r\n"
            )
        })
        .unwrap();
    }

    #[gpui::test]
    fn outline_follows_the_native_caret_across_folded_ancestors(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("caret.json");
        let siblings = |start| {
            (start..start + 20)
                .map(|i| format!(r#""row{i}":0"#))
                .collect::<Vec<_>>()
                .join(",")
        };
        let text = format!(
            "{{{},\r\n\"😀\":{{\"value\":42}},{}}}",
            siblings(0),
            siblings(20)
        );
        std::fs::write(&path, &text).unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root.dispatch(GpuiAction::OutlineToggle, cx);
            root
        });
        cx.refresh().unwrap();
        for selector in [
            "explorer-outline-fold-all",
            "outline-options",
            "outline-follow",
        ] {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let pos = visual.debug_bounds(selector).unwrap().center();
            visual.simulate_click(pos, Default::default());
            cx.refresh().unwrap();
        }
        view.update(cx, |root, _, cx| {
            root.editor_input.get(id).unwrap().update(cx, |state, cx| {
                state.move_to(text.find("42").unwrap(), cx);
            });
        })
        .unwrap();
        cx.refresh().unwrap();
        let child = "outline-name-/1:😀/15:value";
        assert!(
            gpui::VisualTestContext::from_window(view.into(), cx)
                .debug_bounds(child)
                .is_none()
        );
        for selector in ["outline-options", "outline-follow"] {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let pos = visual.debug_bounds(selector).unwrap().center();
            visual.simulate_click(pos, Default::default());
            cx.refresh().unwrap();
        }
        assert!(
            gpui::VisualTestContext::from_window(view.into(), cx)
                .debug_bounds(child)
                .is_some()
        );
        view.update(cx, |root, _, _| {
            let scroll = &root.explorer_input.outline_scroll;
            let row = scroll.bounds_for_item(21).unwrap();
            let viewport = scroll.bounds();
            assert!(viewport.size.height > gpui::px(0.));
            assert!(
                (row.center().y + scroll.offset().y - viewport.center().y).abs() <= gpui::px(1.),
                "row={row:?} viewport={viewport:?} offset={:?}",
                scroll.offset()
            );

            assert_eq!(
                root.app
                    .gpui_editor_document()
                    .unwrap()
                    .tools
                    .outline_selected
                    .as_deref(),
                Some("/1:😀/15:value")
            );
        })
        .unwrap();
    }

    #[gpui::test]
    fn outline_type_filter_preserves_folds_text_and_focus(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("filter.json");
        let text = r#"{"parent":{"target":1},"other":2}"#;
        std::fs::write(&path, text).unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root.dispatch(GpuiAction::OutlineToggle, cx);
            root
        });
        let parent = "outline-name-/1:parent";
        let child = "outline-name-/1:parent/15:target";
        let other = "outline-name-/15:other";
        let click = |cx: &mut TestAppContext, selector| {
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let pos = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"))
                .center();
            visual.simulate_click(pos, Default::default());
        };
        click(cx, "explorer-outline-fold-all");
        click(cx, parent);
        for key in ["t", "g", "t"] {
            cx.simulate_keystrokes(view.into(), key);
            cx.refresh().unwrap();
        }
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_panel_query_input().map(|(_, q)| q),
                Some("tgt")
            );
            assert_eq!(root.app.gpui_editor_document().unwrap().text, text);
        })
        .unwrap();
        {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            assert!(visual.debug_bounds(parent).is_some());
            assert!(visual.debug_bounds(child).is_some());
            assert!(visual.debug_bounds(other).is_none());
        }
        cx.simulate_keystrokes(view.into(), "down");
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "down");
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app
                    .gpui_editor_document()
                    .unwrap()
                    .tools
                    .outline_selected
                    .as_deref(),
                Some("/1:parent/15:target")
            );
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "escape");
        cx.refresh().unwrap();
        {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            assert!(visual.debug_bounds(other).is_some());
            assert!(visual.debug_bounds(child).is_none());
        }
        cx.simulate_keystrokes(view.into(), "z");
        cx.refresh().unwrap();
        assert!(
            gpui::VisualTestContext::from_window(view.into(), cx)
                .debug_bounds("outline-no-results")
                .is_some()
        );
        cx.simulate_keystrokes(view.into(), "escape");
        cx.refresh().unwrap();
        view.update(cx, |root, w, _| {
            assert!(root.app.gpui_panel_query_input().is_none());
            assert!(
                root.explorer_input
                    .outline_focus
                    .get()
                    .unwrap()
                    .is_focused(w)
            );
        })
        .unwrap();
        click(cx, "outline-options");
        click(cx, "outline-filter-mode");
        click(cx, parent);
        cx.simulate_keystrokes(view.into(), "o");
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        assert!(visual.debug_bounds(parent).is_some());
        assert!(visual.debug_bounds(other).is_some());
        assert!(visual.debug_bounds(child).is_none());
        assert!(visual.debug_bounds("outline-no-results").is_none());
    }

    #[gpui::test]
    fn file_timeline_saved_version_opens_outside_git(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("notes.txt");
        std::fs::write(&path, "original\n").unwrap();
        let mut doc = Document::open(&folder, &path).unwrap();
        doc.text = "saved text\n".into();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(folder), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        view.update(cx, |root, _, cx| {
            root.dispatch(
                GpuiAction::Timeline(crate::file_timeline::Action::Toggle),
                cx,
            );
            root.dispatch(GpuiAction::SaveActiveDocument, cx);
        })
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved text\n");
        std::fs::write(&path, "current text\n").unwrap();
        let click = |cx: &mut TestAppContext, selector: &'static str| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            loop {
                let loaded = view
                    .update(cx, |root, _, cx| {
                        if root.poll_backend(cx) {
                            cx.notify();
                        }
                        root.app
                            .gpui_snapshot(960., 640., super::super::theme::SPLIT_DIVIDER_PX)
                            .side_panels
                            .timeline
                            .data
                            .is_some()
                    })
                    .unwrap();
                cx.refresh().unwrap();
                if loaded {
                    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
                    let point = visual
                        .debug_bounds(selector)
                        .unwrap_or_else(|| panic!("missing {selector}"))
                        .center();
                    visual.simulate_click(point, Default::default());
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timeline did not load before {selector}"
                );
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
        };
        for selector in [
            "timeline-filter",
            "timeline-filter-git",
            "timeline-filter",
            "timeline-filter-local",
        ] {
            click(cx, selector);
        }
        view.update(cx, |root, _, _| {
            let timeline = root
                .app
                .gpui_snapshot(960., 640., super::super::theme::SPLIT_DIVIDER_PX)
                .side_panels
                .timeline;
            assert!(!timeline.filters_open);
            assert_eq!(
                timeline.status(false).as_deref(),
                Some("All timeline sources have been filtered out.")
            );
        })
        .unwrap();
        click(cx, "timeline-filter");
        click(cx, "timeline-filter-local");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            view.update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
            })
            .unwrap();
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            if let Some(bounds) = visual.debug_bounds("timeline-entry-0") {
                visual.simulate_click(bounds.center(), Default::default());
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "saved version missing"
            );
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        loop {
            let opened = view
                .update(cx, |root, _, cx| {
                    root.poll_backend(cx);
                    root.app.gpui_editor_document().is_some_and(|d| {
                        d.diff_title.is_some()
                            && d.text.contains("-saved text")
                            && d.text.contains("+current text")
                    })
                })
                .unwrap();
            if opened {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "saved version diff did not open"
            );
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
    }

    #[gpui::test]
    fn file_timeline_opens_clicked_commit_as_a_diff(cx: &mut TestAppContext) {
        exercise_timeline_diff(cx, false);
    }

    #[gpui::test]
    fn file_timeline_pin_survives_tab_switch_and_unpins(cx: &mut TestAppContext) {
        exercise_timeline_diff(cx, true);
    }

    fn exercise_timeline_diff(cx: &mut TestAppContext, pin: bool) {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("repo");
        std::fs::create_dir(&folder).unwrap();
        let git = |args: &[&str]| {
            let mut options = vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
            ];
            options.extend_from_slice(args);
            crate::git::run(&folder, &options, None).unwrap()
        };
        git(&["init", "-q"]);
        if pin {
            git(&[
                "remote",
                "add",
                "origin",
                "https://github.com/example/fixture.git",
            ]);
        }
        let path = folder.join("history.txt");
        std::fs::write(&path, "timeline fixture\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "File history fixture"]);
        let hash = git(&["rev-parse", "HEAD"]).trim().to_owned();
        let doc = Document::open(&folder, &path).unwrap();
        let other = folder.join("other.txt");
        std::fs::write(&other, "another tab").unwrap();
        let other_doc = Document::open(&folder, &other).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(folder), cx);
            root.app
                .install_editor_input_fixture(vec![doc, other_doc], 0);
            root
        });
        cx.refresh().unwrap();
        {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let button = visual.debug_bounds("timeline-toggle").unwrap().center();
            visual.simulate_click(button, Default::default());
        }
        if pin {
            cx.refresh().unwrap();
            {
                let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
                let pin = visual.debug_bounds("timeline-pin").unwrap().center();
                visual.simulate_click(pin, Default::default());
            }
            view.update(cx, |root, _, cx| {
                root.dispatch(GpuiAction::SelectDocumentTab(1), cx)
            })
            .unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let title = format!("history.txt ({})", &hash[..7]);
        loop {
            view.update(cx, |root, _, cx| {
                if root.poll_backend(cx) {
                    cx.notify();
                }
            })
            .unwrap();
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            if let Some(bounds) = visual.debug_bounds("timeline-entry-0") {
                let position = bounds.center();
                visual.simulate_mouse_move(position, None, Default::default());
                cx.run_until_parked();
                cx.executor()
                    .advance_clock(std::time::Duration::from_millis(550));
                cx.run_until_parked();
                cx.refresh().unwrap();
                let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
                assert!(
                    visual.debug_bounds("timeline-hover").is_some(),
                    "commit tooltip missing"
                );
                loop {
                    cx.executor()
                        .advance_clock(std::time::Duration::from_millis(80));
                    cx.run_until_parked();
                    cx.refresh().unwrap();
                    let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
                    if visual.debug_bounds("timeline-hover-stats").is_some() {
                        let copy = visual.debug_bounds("timeline-copy-hash").unwrap().center();
                        visual.simulate_mouse_move(copy, None, Default::default());
                        visual.simulate_click(copy, Default::default());
                        assert_eq!(
                            cx.update(|cx| cx.read_from_clipboard().and_then(|v| v.text()))
                                .as_deref(),
                            Some(hash.as_str())
                        );
                        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
                        assert_eq!(visual.debug_bounds("timeline-open-github").is_some(), pin);
                        let open = if pin {
                            visual
                                .debug_bounds("timeline-open-commit")
                                .unwrap()
                                .center()
                        } else {
                            bounds.center()
                        };
                        visual.simulate_mouse_move(open, None, Default::default());
                        visual.simulate_click(open, Default::default());
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "commit statistics missing"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(15));
                }
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "commit row did not appear"
            );
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        loop {
            let opened = view
                .update(cx, |root, _, cx| {
                    root.poll_backend(cx);
                    root.app.gpui_editor_document().is_some_and(|doc| {
                        doc.diff_title.as_ref() == Some(&title)
                            && doc.text.contains("+timeline fixture")
                    })
                })
                .unwrap();
            if opened {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "click did not open commit diff"
            );
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        if pin {
            view.update(cx, |root, _, cx| {
                root.dispatch(GpuiAction::SelectDocumentTab(1), cx)
            })
            .unwrap();
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let button = visual.debug_bounds("timeline-pin").unwrap().center();
            visual.simulate_click(button, Default::default());
            view.update(cx, |root, _, _| {
                let timeline = root
                    .app
                    .gpui_snapshot(960., 640., super::super::theme::SPLIT_DIVIDER_PX)
                    .side_panels
                    .timeline;
                assert!(!timeline.pinned);
                assert_eq!(timeline.file.unwrap().file_name().unwrap(), "other.txt");
            })
            .unwrap();
        }
    }

    #[gpui::test]
    fn outline_options_sort_visible_rows(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sort.json");
        std::fs::write(&path, r#"{"z":3,"a":true,"b":"s"}"#).unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root.dispatch(GpuiAction::OutlineToggle, cx);
            root
        });
        let a = "outline-name-/16:a";
        let b = "outline-name-/14:b";
        let z = "outline-name-/15:z";
        for (choice, order) in [
            ("Sort By: Name", [a, b, z]),
            ("Sort By: Category", [b, z, a]),
            ("Sort By: Position", [z, a, b]),
        ] {
            for selector in ["outline-options", choice] {
                cx.refresh().unwrap();
                let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
                let point = visual.debug_bounds(selector).unwrap().center();
                visual.simulate_click(point, Default::default());
            }
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let [first, second, third] =
                order.map(|row| visual.debug_bounds(row).unwrap().center().y);
            assert!(first < second && second < third, "{choice}");
            assert!(visual.debug_bounds(choice).is_none());
        }
    }

    #[gpui::test]
    fn outline_full_range_returns_keyboard_to_editor(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("focus.json");
        std::fs::write(&path, r#"{"😀":[1],"keep":2}"#).unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root.dispatch(GpuiAction::OutlineToggle, cx);
            root
        });
        cx.refresh().unwrap();
        for children_visible in [false, true] {
            {
                let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
                let button = visual
                    .debug_bounds("explorer-outline-fold-all")
                    .unwrap()
                    .center();
                visual.simulate_click(button, Default::default());
            }
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            assert_eq!(
                visual.debug_bounds("outline-name-/17:😀/15:0").is_some(),
                children_visible
            );
        }
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-f");
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(!root.editor_input.focused(window, cx));
        })
        .unwrap();
        {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let position = visual.debug_bounds("outline-name-/17:😀").unwrap().center();
            visual.simulate_mouse_move(position, None, Default::default());
            visual.simulate_click(position, Default::default());
            visual.simulate_event(gpui::MouseDownEvent {
                position,
                modifiers: Default::default(),
                button: gpui::MouseButton::Left,
                click_count: 2,
                first_mouse: false,
            });
            visual.simulate_event(gpui::MouseUpEvent {
                position,
                modifiers: Default::default(),
                button: gpui::MouseButton::Left,
                click_count: 2,
            });
        }
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.editor_input.focused(window, cx))
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "x");
        view.update(cx, |root, _, _| {
            assert_eq!(
                root.app.gpui_editor_document().unwrap().text,
                r#"{x,"keep":2}"#
            )
        })
        .unwrap();
    }

    #[gpui::test]
    fn native_editor_restores_focus_without_append_fallback(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("focus.txt");
        std::fs::write(&path, "abc").unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, window, cx| {
            assert!(root.editor_input.focused(window, cx));
            let input = root.editor_input.get(id).unwrap().clone();
            input.update(cx, |state, cx| state.move_to(1, cx));
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "x");
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "axbc")
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-f");
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(!root.editor_input.focused(window, cx))
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "escape");
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "y");
        view.update(cx, |root, window, cx| {
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "axybc");
            window.focus(&root.focus_handle, cx);
            assert!(!root.app.gpui_handle_text_target_key(
                super::super::FocusInputTarget::EditorBody,
                "z",
                Some("z"),
                false,
                None
            ));
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "axybc");
        })
        .unwrap();
        view.update(cx, |root, _, cx| {
            root.focus_input = super::super::FocusInputTarget::None;
            root.app.set_floating_input_fixture(true);
            cx.notify();
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert_eq!(
                root.effective_focus_target(),
                super::super::FocusInputTarget::Terminal(None)
            );
            assert!(!root.editor_input.focused(window, cx));
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "x");
        view.update(cx, |root, _, cx| {
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "axybc");
            root.app.set_floating_input_fixture(false);
            cx.notify();
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.editor_input.focused(window, cx))
        })
        .unwrap();
    }

    #[gpui::test]
    fn native_editor_navigates_unicode_beyond_preview_limit(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.txt");
        let source = "😀 é\n".repeat(600);
        std::fs::write(&path, &source).unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::SetEditorGotoSpec("600:3".into()), cx);
            root.dispatch(GpuiAction::ApplyEditorGoto, cx);
            // Layout uses multiple snapshots; none may consume the navigation request.
            root.app
                .gpui_snapshot(960., 640., super::super::theme::SPLIT_DIVIDER_PX);
            root.app
                .gpui_snapshot(960., 640., super::super::theme::SPLIT_DIVIDER_PX);
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, window, cx| {
            let input = root.editor_input.get(id).unwrap().clone();
            assert_eq!(input.read(cx).as_str(), source);
            input.update(cx, |state, cx| {
                assert_eq!(
                    state.selected_text_range(false, window, cx).unwrap().range,
                    2998..2998
                );
                state.replace_text_in_range(None, "Z", window, cx);
            });
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, cx| {
            assert!(
                root.app
                    .gpui_editor_document()
                    .unwrap()
                    .text
                    .ends_with("😀 Zé\n")
            );
            root.dispatch(GpuiAction::SetEditorFindQuery("é".into()), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, window, cx| {
            root.editor_input.get(id).unwrap().update(cx, |state, cx| {
                assert_eq!(
                    state.selected_text_range(false, window, cx).unwrap().range,
                    3..4
                );
            });
        })
        .unwrap();
    }

    #[gpui::test]
    fn native_editor_keeps_per_tab_history_and_rejects_old_entities(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("one.rs");
        std::fs::write(&path, "let a = \"é😀\";\r\n// end\n").unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let readonly = Document::from_readonly_text(
            dir.path().into(),
            dir.path().join("readonly"),
            "read only".into(),
            "fixed".into(),
            false,
        );
        let readonly_id = readonly.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app
                .install_editor_input_fixture(vec![doc, readonly], 0);
            root
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let input = view
            .update(cx, |root, window, cx| {
                let input = root.editor_input.get(id).unwrap().clone();
                window.focus(&input.read(cx).focus_handle(cx), cx);
                input
            })
            .unwrap();
        view.update(cx, |_, window, cx| {
            input.update(cx, |state, cx| {
                state.replace_text_in_range(Some(4..5), "b", window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, cx| {
            assert!(
                root.app
                    .gpui_editor_document()
                    .unwrap()
                    .text
                    .starts_with("let b")
            );
            root.dispatch(GpuiAction::SelectDocumentTab(1), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        view.update(cx, |root, window, cx| {
            let readonly = root.editor_input.get(readonly_id).unwrap().clone();
            readonly.update(cx, |state, cx| {
                state.replace_text_in_range(Some(0..5), "bad", window, cx)
            });
            input.update(cx, |state, cx| {
                state.replace_text_in_range(Some(0..3), "old", window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, cx| {
            assert_eq!(root.app.gpui_editor_document().unwrap().text, "fixed");
            root.dispatch(GpuiAction::SelectDocumentTab(0), cx);
        })
        .unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
        let current = view
            .update(cx, |root, window, cx| {
                assert!(
                    root.app
                        .gpui_editor_document()
                        .unwrap()
                        .text
                        .starts_with("let b")
                );
                let current = root.editor_input.get(id).unwrap().clone();
                window.focus(&current.read(cx).focus_handle(cx), cx);
                current
            })
            .unwrap();
        // An inactive entity's rejected edit is restored, not replayed into the reopened tab.
        assert_ne!(current, input);
        view.update(cx, |_, window, cx| {
            current.update(cx, |state, cx| {
                state.set_selected_text_range(4..5, window, cx);
                state.paste(gpui::ClipboardItem::new_string("名".into()), window, cx);
            })
        })
        .unwrap();
        cx.run_until_parked();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Undo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert!(
                root.app
                    .gpui_editor_document()
                    .unwrap()
                    .text
                    .starts_with("let b")
            )
        })
        .unwrap();
        cx.dispatch_action(view.into(), gpui_ce_elements::editable_text::actions::Redo);
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert!(
                root.app
                    .gpui_editor_document()
                    .unwrap()
                    .text
                    .starts_with("let 名")
            )
        })
        .unwrap();
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::SelectDocumentTab(1), cx)
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::SelectDocumentTab(0), cx)
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            assert_eq!(root.editor_input.get(id).unwrap(), &current)
        })
        .unwrap();
        // Close/reopen the same filename: the old identity must not survive.
        view.update(cx, |root, _, cx| {
            root.app
                .install_editor_input_fixture(vec![Document::open(dir.path(), &path).unwrap()], 0);
            cx.notify();
        })
        .unwrap();
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            assert!(root.editor_input.get(id).is_none());
            current.update(cx, |state, cx| {
                state.replace_text_in_range(Some(0..3), "bad", window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            assert!(
                root.app
                    .gpui_editor_document()
                    .unwrap()
                    .text
                    .starts_with("let a")
            )
        })
        .unwrap();
    }
}

#[derive(Default)]
pub(crate) struct Input {
    pub tools: super::editor_tool_inputs::Inputs,
    bindings: HashMap<u64, Binding>,
    pub syntax: editor_syntax::Cache,
    styled: Option<editor_syntax::StyledSource>,
}

impl Input {
    pub fn focused(&self, window: &Window, cx: &App) -> bool {
        self.bindings
            .values()
            .any(|binding| binding.state.read(cx).focus_handle(cx).is_focused(window))
    }

    pub fn get(&self, id: u64) -> Option<&Entity<EditableTextState>> {
        self.bindings.get(&id).map(|binding| &binding.state)
    }

    pub fn prepare_styles(
        &mut self,
        doc: &GpuiActiveDocumentSummary,
        theme: crate::theme_palettes::ThemeId,
        dark: bool,
    ) {
        self.styled = Some(self.syntax.update(
            &doc.text,
            if doc.syntax_highlighting {
                doc.lang
            } else {
                crate::syntax::SyntaxLang::Plain
            },
            theme,
            dark,
            doc.is_diff,
            &doc.match_lines,
        ));
    }

    pub fn render(
        &self,
        doc: &GpuiActiveDocumentSummary,
        font_size: f32,
        tokens: &super::theme::GpuiThemeTokens,
        cx: &mut Context<TesselGpuiRoot>,
    ) -> Option<gpui::AnyElement> {
        use gpui::{
            Bounds, EntityInputHandler, InteractiveElement, IntoElement, ParentElement,
            StatefulInteractiveElement, Styled, TextAlign, TextRun, canvas, div, point,
            prelude::FluentBuilder, px, size,
        };
        use gpui_ce_elements::editable_text::text_area;
        let input = self.get(doc.instance_id)?.clone();
        let styled = self.styled.as_ref()?.clone();
        let line_height = px((font_size * 1.42).round().max(15.));
        let show_numbers = doc.line_numbers;
        let gutter_width = if show_numbers {
            64.
        } else if doc.hunk_lines.is_empty() {
            10.
        } else {
            16.
        };
        let build = || text_area("native-local-editor").state(input.downgrade());
        let area = build()
            .debug_selector(|| "native-local-editor".to_owned())
            .styled_runs(styled.source, styled.runs)
            .unwrap_or_else(|_| build())
            .accepts_input(!doc.media)
            .key_context("EditableText TesselLocalEditor")
            .on_action(
                cx.listener(|root, _: &Indent, window, cx| apply_indent(root, false, window, cx)),
            )
            .on_action(
                cx.listener(|root, _: &Outdent, window, cx| apply_indent(root, true, window, cx)),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|root, _, _, _| root.focus_input = super::FocusInputTarget::EditorBody),
            )
            .size_full()
            .pl(px(gutter_width))
            .pr(px(10.))
            .py(px(8.))
            .overflow_y_scroll()
            .when(!doc.word_wrap, |area| {
                area.whitespace_nowrap().overflow_x_scroll()
            })
            .font_family(super::theme::mono_font_family())
            .text_size(px(font_size))
            .line_height(line_height)
            .text_color(tokens.text);
        let ink = gpui::rgb_to_hsla(tokens.text_dim);
        let marker = tokens.accent;
        let gutter_background = tokens.term;
        let hunks: std::collections::HashSet<_> = doc.hunk_lines.iter().copied().collect();
        let line_starts = styled.utf16_lines;
        let gutter = canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                let inner = Bounds::new(
                    bounds.origin + point(px(gutter_width), px(8.)),
                    size(
                        (bounds.size.width - px(gutter_width + 10.)).max(px(0.)),
                        (bounds.size.height - px(16.)).max(px(0.)),
                    ),
                );
                let (first, last) = input.update(cx, |state, cx| {
                    let first = state
                        .character_index_for_point(inner.origin, window, cx)
                        .unwrap_or(0);
                    let last = state
                        .character_index_for_point(
                            inner.origin + point(px(0.), inner.size.height),
                            window,
                            cx,
                        )
                        .unwrap_or(first);
                    (
                        line_starts
                            .partition_point(|offset| *offset <= first)
                            .saturating_sub(1),
                        line_starts.partition_point(|offset| *offset <= last),
                    )
                });
                window.with_content_mask(
                    Some(gpui::ContentMask {
                        bounds,
                        ..Default::default()
                    }),
                    |window| {
                        window.paint_quad(
                            gpui::PaintQuad::new(Bounds::new(
                                bounds.origin,
                                size(px(gutter_width), bounds.size.height),
                            ))
                            .background(gutter_background),
                        );
                        for index in first..last.min(line_starts.len()) {
                            let offset = line_starts[index];
                            let position = input.update(cx, |state, cx| {
                                state.bounds_for_range(offset..offset, inner, window, cx)
                            });
                            let Some(position) = position else { continue };
                            if position.origin.y + line_height < bounds.origin.y
                                || position.origin.y > bounds.bottom()
                            {
                                continue;
                            }
                            if show_numbers {
                                let label: gpui::SharedString = (index + 1).to_string().into();
                                let run = TextRun {
                                    len: label.len(),
                                    font: gpui::font(super::theme::mono_font_family()),
                                    color: ink,
                                    ..Default::default()
                                };
                                let line =
                                    window
                                        .text_system()
                                        .shape_line(label, px(font_size), &[run]);
                                let _ = line.paint(
                                    point(bounds.origin.x + px(10.), position.origin.y),
                                    line_height,
                                    TextAlign::Right,
                                    Some(px(40.)),
                                    window,
                                    cx,
                                );
                            }
                            if hunks.contains(&index) {
                                window.paint_quad(
                                    gpui::PaintQuad::new(Bounds::new(
                                        point(bounds.origin.x + px(3.), position.origin.y),
                                        size(px(3.), line_height),
                                    ))
                                    .background(marker),
                                );
                            }
                        }
                    },
                );
            },
        )
        .absolute()
        .size_full();
        Some(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .bg(tokens.term)
                .font_family(super::theme::mono_font_family())
                .text_size(px(font_size))
                .line_height(line_height)
                .child(area)
                .child(gutter)
                .into_any_element(),
        )
    }

    pub fn sync(
        &mut self,
        doc: Option<&GpuiActiveDocumentSummary>,
        open: &[u64],
        window: &mut Window,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        let restore_focus = self.focused(window, cx);
        self.bindings.retain(|id, _| open.contains(id));
        let Some(doc) = doc else { return };
        let id = doc.instance_id;
        if let Some(binding) = self.bindings.get(&id)
            && binding.state.read(cx).as_str() == doc.text
        {
            if restore_focus {
                window.focus(&binding.state.read(cx).focus_handle(cx), cx);
            }
            return;
        }
        // External replace/reload or rejected edits retire the prior undo history.
        let state = cx.new(|cx| EditableTextState::new(StringStorage::from(doc.text.as_str()), cx));
        if restore_focus {
            window.focus(&state.read(cx).focus_handle(cx), cx);
        }
        let changed = cx.subscribe(&state, move |root, state, _: &TextChanged, cx| {
            let Some(binding) = root.editor_input.bindings.get(&id) else {
                return;
            };
            if binding.state != state {
                return;
            }
            let Some(doc) = root.app.gpui_editor_document() else {
                return;
            };
            if doc.instance_id() != id || doc.media {
                return;
            }
            let base = binding.synced.clone();
            let text = state.read(cx).as_str().to_owned();
            if text == base {
                return;
            }
            root.dispatch(
                GpuiAction::ApplyNativeDocumentEdit {
                    instance_id: id,
                    base,
                    text,
                },
                cx,
            );
            if let Some(doc) = root.app.gpui_editor_document()
                && doc.instance_id() == id
                && let Some(binding) = root.editor_input.bindings.get_mut(&id)
            {
                binding.synced.clone_from(&doc.text);
            }
        });
        let caret = cx.observe(&state, move |root, state, cx| {
            if root
                .editor_input
                .bindings
                .get(&id)
                .is_some_and(|b| b.state == state)
                && root
                    .app
                    .record_outline_cursor(id, state.read(cx).caret_byte_offset())
            {
                cx.notify();
            }
        });
        self.bindings.insert(
            id,
            Binding {
                state,
                synced: doc.text.clone(),
                _changed: changed,
                _caret: caret,
                indentation: editor_indent::Options::detect(&doc.text),
            },
        );
    }
}
