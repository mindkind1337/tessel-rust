//! Native editor entities are retained per open document, never per filename or tab index.
use super::{TesselGpuiRoot, editor_indent, editor_syntax};
use crate::app::gpui_bridge::{GpuiAction, GpuiActiveDocumentSummary};
use gpui::{App, AppContext, Context, Entity, Focusable, Subscription, Window};
use gpui_ce_elements::editable_text::{EditableTextState, StringStorage, TextChanged};
use std::collections::HashMap;

struct Binding {
    state: Entity<EditableTextState>,
    synced: String,
    disk_reload: u64,
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
    fn viewer_copies_the_open_file_path(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file with spaces.txt");
        std::fs::write(&path, "fixture").unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let point = visual.debug_bounds("viewer-copy-path").unwrap().center();
        visual.simulate_click(point, Default::default());
        assert_eq!(
            cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
            Some(dunce::simplified(&path).display().to_string())
        );
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
        click(cx, "outline-fold-/1:parent");
        {
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            assert!(visual.debug_bounds(child).is_none());
            let name = visual.debug_bounds(parent).unwrap();
            let row = visual.debug_bounds("outline-symbol-/1:parent").unwrap();
            let pos = gpui::point(row.right() - gpui::px(3.), row.center().y);
            assert!(pos.x > name.right());
            visual.simulate_click(pos, Default::default());
        }
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
        git(&[
            "commit",
            "-qm",
            "File history fixture",
            "-m",
            "- see `details` in #42",
        ]);
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
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::SelectWorkTab, cx);
            let timeline = root.app.gpui_snapshot(960., 640., 4.).side_panels.timeline;
            assert_eq!(timeline.file.unwrap().file_name().unwrap(), "history.txt");
            assert_eq!(timeline.pinned, pin);
            if pin {
                root.dispatch(GpuiAction::SelectBrowserTab, cx);
            }
        })
        .unwrap();
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
                        assert!(visual.debug_bounds("timeline-email-author").is_some());
                        assert!(visual.debug_bounds("timeline-message-bullet").is_some());
                        assert_eq!(visual.debug_bounds("timeline-inline-links").is_some(), pin);
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
    fn html_outline_click_leaves_preview_and_reveals_the_source(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.html");
        std::fs::write(&path, "<main>😀<p id=one>hello</p></main>").unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        assert!(doc.preview_mode);
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root.dispatch(GpuiAction::OutlineToggle, cx);
            root
        });
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let pos = visual
            .debug_bounds("outline-name-/7:main/7:p#one")
            .unwrap()
            .center();
        visual.simulate_click(pos, Default::default());
        cx.refresh().unwrap();
        view.update(cx, |root, _, _| {
            let doc = root.app.gpui_editor_document().unwrap();
            assert!(!doc.preview_mode);
            assert_eq!(
                doc.tools.outline_selected.as_deref(),
                Some("/7:main/7:p#one")
            );
        })
        .unwrap();
    }

    #[gpui::test]
    fn css_outline_selects_the_rule_in_the_live_editor_buffer(cx: &mut TestAppContext) {
        stylesheet_selection(
            cx,
            "style.CSS",
            "/*😀*/ @media print {h1, .title {color:red}}",
            "outline-name-/1:@media print/4:.title",
            "/*😀*/ @media print {h1, x}",
        );
    }

    #[gpui::test]
    fn scss_outline_selects_the_variable_in_the_live_editor_buffer(cx: &mut TestAppContext) {
        stylesheet_selection(
            cx,
            "style.scss",
            "@mixin paint { $accent: red; }",
            "outline-name-/5:paint/12:$accent",
            "@mixin paint { x; }",
        );
    }

    #[gpui::test]
    fn less_outline_selects_the_variable_in_the_live_editor_buffer(cx: &mut TestAppContext) {
        stylesheet_selection(
            cx,
            "style.less",
            ".paint() { @accent: red; }",
            "outline-name-/5:.paint/12:@accent",
            ".paint() { x; }",
        );
    }

    fn stylesheet_selection(
        cx: &mut TestAppContext,
        filename: &str,
        draft: &str,
        symbol: &'static str,
        expected: &str,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(filename);
        std::fs::write(&path, "body {}").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text = draft.into();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root.dispatch(GpuiAction::OutlineToggle, cx);
            root
        });
        cx.refresh().unwrap();
        let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
        let position = visual.debug_bounds(symbol).unwrap().center();
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
        cx.refresh().unwrap();
        cx.simulate_keystrokes(view.into(), "x");
        view.update(cx, |root, _, _| {
            assert_eq!(root.app.gpui_editor_document().unwrap().text, expected);
        })
        .unwrap();
    }

    #[gpui::test]
    fn disk_conflict_buttons_compare_then_reload_the_original_tab(cx: &mut TestAppContext) {
        cx.update(|cx| {
            use gpui_ce_elements::editable_text::actions::{
                DEFAULT_INPUT_CONTEXT, default_bindings,
            };
            cx.bind_keys(default_bindings().as_keybindings(Some(DEFAULT_INPUT_CONTEXT)));
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.txt");
        std::fs::write(&path, "base").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        doc.text = "my draft".into();
        std::fs::write(&path, "\u{feff}disk edit").unwrap();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root.dispatch(GpuiAction::SaveActiveDocument, cx);
            root
        });
        let click = |cx: &mut TestAppContext, selector| {
            cx.refresh().unwrap();
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            let pos = visual.debug_bounds(selector).unwrap().center();
            visual.simulate_click(pos, Default::default());
        };
        click(cx, "disk-conflict-compare");
        view.update(cx, |root, _, _| {
            let doc = root.app.gpui_editor_document().unwrap();
            assert_eq!(doc.text, "my draft");
            let comparison = doc.comparison().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while comparison.patch.get().is_none() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            let patch = comparison.patch.get().unwrap();
            assert!((0..patch.line_count()).any(|i| patch.line(i).starts_with("-disk edit")));
            assert!((0..patch.line_count()).any(|i| patch.line(i).starts_with("+my draft")));
        })
        .unwrap();
        click(cx, "disk-conflict-reload");
        cx.refresh().unwrap();
        view.update(cx, |root, _, cx| {
            let doc = root.app.gpui_editor_document().unwrap();
            assert_eq!(doc.instance_id(), id);
            assert!(!doc.dirty());
            assert_eq!(
                root.editor_input.get(id).unwrap().read(cx).as_str(),
                "disk edit"
            );
        })
        .unwrap();
        view.update(cx, |root, window, cx| {
            let input = root.editor_input.get(id).unwrap().read(cx);
            window.focus(&input.focus_handle(cx), cx);
        })
        .unwrap();
        cx.simulate_keystrokes(view.into(), "ctrl-z");
        cx.run_until_parked();
        view.update(cx, |root, _, _| {
            let doc = root.app.gpui_editor_document().unwrap();
            assert_eq!(doc.text, "my draft");
            assert!(doc.dirty());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "\u{feff}disk edit");
        })
        .unwrap();
    }

    #[gpui::test]
    fn outline_breadcrumb_follows_caret_with_panel_closed_and_reveals_parent(
        cx: &mut TestAppContext,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("nested");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("trail.json");
        let text = r#"{"parent":{"leaf":42}}"#;
        std::fs::write(&path, text).unwrap();
        let doc = Document::open(dir.path(), &path).unwrap();
        let id = doc.instance_id();
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.dispatch(GpuiAction::AddProjectPath(dir.path().to_owned()), cx);
            root.app.install_editor_input_fixture(vec![doc], 0);
            root
        });
        cx.refresh().unwrap();
        view.update(cx, |root, _, cx| {
            root.editor_input.get(id).unwrap().update(cx, |state, cx| {
                state.move_to(text.find("42").unwrap(), cx);
            });
        })
        .unwrap();
        cx.refresh().unwrap();
        {
            let mut visual = gpui::VisualTestContext::from_window(view.into(), cx);
            assert!(visual.debug_bounds("editor-crumb-1").is_some());
            let pos = visual.debug_bounds("editor-crumb-0").unwrap().center();
            visual.simulate_click(pos, Default::default());
        }
        cx.refresh().unwrap();
        view.update(cx, |root, window, cx| {
            let snapshot = root.app.gpui_snapshot(800., 600., 2.);
            assert_eq!(
                snapshot.active_document.unwrap().breadcrumb_path,
                ["nested", "trail.json"]
            );
            assert!(root.editor_input.focused(window, cx));
            assert_eq!(
                root.editor_input
                    .get(id)
                    .unwrap()
                    .read(cx)
                    .caret_byte_offset(),
                1
            );
            assert_eq!(root.app.gpui_editor_document().unwrap().text, text);
        })
        .unwrap();
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
    fn editor_font_size_changes_source_and_diff_metrics_independently(test: &mut TestAppContext) {
        let platform = gpui_platform::current_platform(true);
        let cx = &mut TestAppContext::build_with_text_system(
            test.dispatcher.clone(),
            None,
            platform.text_system(),
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("size.txt");
        std::fs::write(&path, "abcdefghij").unwrap();
        let docs = vec![
            Document::open(dir.path(), &path).unwrap(),
            Document::from_diff(dir.path(), path, "Diff".into(), "+abcdefghij".into(), false),
        ];
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(dir.path().join("state")), false, cx);
            root.app.install_editor_input_fixture(docs, 0);
            root
        });
        let width = |cx: &mut TestAppContext, tab| {
            view.update(cx, |root, _, cx| {
                root.dispatch(GpuiAction::SelectDocumentTab(tab), cx)
            })
            .unwrap();
            cx.refresh().unwrap();
            let bounds = gpui::VisualTestContext::from_window(view.into(), cx)
                .debug_bounds("native-local-editor")
                .unwrap();
            view.update(cx, |root, w, cx| {
                let id = root.app.gpui_editor_document().unwrap().instance_id();
                root.editor_input.get(id).unwrap().update(cx, |s, cx| {
                    let start = s.bounds_for_range(0..0, bounds, w, cx).unwrap();
                    let end = s.bounds_for_range(10..10, bounds, w, cx).unwrap();
                    f32::from(end.origin.x - start.origin.x)
                })
            })
            .unwrap()
        };
        let original = [width(cx, 0), width(cx, 1)];
        let terminal = view
            .update(cx, |root, _, cx| {
                let snap = root.app.gpui_snapshot(960., 640., 4.);
                assert_eq!(snap.editor_font_size, 12);
                root.dispatch(GpuiAction::SetEditorFontSize { base: 12, size: 24 }, cx);
                snap.font_size
            })
            .unwrap();
        for (tab, baseline) in original.into_iter().enumerate() {
            assert!(baseline > 0. && width(cx, tab) > baseline * 1.8);
        }
        let saved: crate::workspace::Workspace = serde_json::from_slice(
            &std::fs::read(dir.path().join("state/workspace.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved.editor_prefs.font_size, 24);
        assert_eq!(saved.font_size, terminal);
        let previous: crate::workspace::EditorPreferences = serde_json::from_str("{}").unwrap();
        assert_eq!(previous.font_size, 12);
    }

    #[gpui::test]
    fn source_wrap_is_shared_live_and_persisted_independently_of_diff(test: &mut TestAppContext) {
        // The default GPUI test text system gives each paragraph exactly one visual line.
        let platform = gpui_platform::current_platform(true);
        let cx = &mut TestAppContext::build_with_text_system(
            test.dispatcher.clone(),
            None,
            platform.text_system(),
        );
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().join("state");
        let mut docs = Vec::new();
        let text = "long line ".repeat(70);
        for name in ["first.txt", "second.txt"] {
            let path = dir.path().join(name);
            std::fs::write(&path, &text).unwrap();
            docs.push(Document::open(dir.path(), &path).unwrap());
        }
        docs.push(Document::from_diff(
            dir.path(),
            dir.path().join("first.txt"),
            "Diff".into(),
            format!("+{text}"),
            false,
        ));
        let view = cx.add_window(|_, cx| {
            let mut root = TesselGpuiRoot::new_in_dir(Some(state_dir.clone()), false, cx);
            root.app.install_editor_input_fixture(docs, 0);
            root
        });
        let line_wraps = |cx: &mut TestAppContext| {
            cx.refresh().unwrap();
            let bounds = gpui::VisualTestContext::from_window(view.into(), cx)
                .debug_bounds("native-local-editor")
                .unwrap();
            view.update(cx, |root, w, cx| {
                let doc = root.app.gpui_editor_document().unwrap();
                let end = doc.text.len();
                root.editor_input
                    .get(doc.instance_id())
                    .unwrap()
                    .update(cx, |s, cx| {
                        let first = s.bounds_for_range(0..0, bounds, w, cx).unwrap();
                        let last = s.bounds_for_range(end..end, bounds, w, cx).unwrap();
                        last.origin.y > first.origin.y
                    })
            })
            .unwrap()
        };
        assert!(line_wraps(cx));
        cx.simulate_keystrokes(view.into(), "alt-z");
        assert!(!line_wraps(cx));
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::SelectDocumentTab(1), cx)
        })
        .unwrap();
        assert!(!line_wraps(cx));
        let persisted = || {
            let bytes = std::fs::read(state_dir.join("workspace.json")).unwrap();
            serde_json::from_slice::<crate::workspace::Workspace>(&bytes)
                .unwrap()
                .editor_prefs
        };
        let mut prefs = persisted();
        assert!(!prefs.word_wrap);
        prefs.word_wrap = true;
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::UpdateEditorPrefs(prefs), cx)
        })
        .unwrap();
        assert!(line_wraps(cx));
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::SelectDocumentTab(2), cx)
        })
        .unwrap();
        assert!(!line_wraps(cx));
        prefs.diff_word_wrap = true;
        view.update(cx, |root, _, cx| {
            root.dispatch(GpuiAction::UpdateEditorPrefs(prefs), cx)
        })
        .unwrap();
        assert!(line_wraps(cx));
        assert_eq!(persisted(), prefs);
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
        let wrapped = view
            .update(cx, |root, _, _| {
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .active_document
                    .unwrap()
                    .word_wrap
            })
            .unwrap();
        cx.simulate_keystrokes(view.into(), "alt-z");
        view.update(cx, |root, _, _| {
            assert_ne!(
                root.app
                    .gpui_snapshot(960., 640., 4.)
                    .active_document
                    .unwrap()
                    .word_wrap,
                wrapped
            )
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
    pub(super) guest: super::guest_document_input::Input,
    pub tools: super::editor_tool_inputs::Inputs,
    bindings: HashMap<u64, Binding>,
    plain: Option<(std::sync::Arc<str>, Entity<EditableTextState>)>,
    pub syntax: editor_syntax::Cache,
    styled: Option<editor_syntax::StyledSource>,
}

impl Input {
    pub fn sync_plain(
        &mut self,
        text: Option<&std::sync::Arc<str>>,
        cx: &mut Context<TesselGpuiRoot>,
    ) {
        if let Some(text) = text {
            if !self
                .plain
                .as_ref()
                .is_some_and(|(previous, _)| std::sync::Arc::ptr_eq(previous, text))
            {
                let state =
                    cx.new(|cx| EditableTextState::new(StringStorage::from(text.as_ref()), cx));
                self.plain = Some((text.clone(), state));
            }
        } else {
            self.plain = None;
        }
    }

    pub fn plain(&self) -> Option<&Entity<EditableTextState>> {
        self.plain.as_ref().map(|(_, state)| state)
    }

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
            .when(doc.word_wrap, |area| area.flex_col().whitespace_normal())
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
        if let Some(binding) = self.bindings.get_mut(&id) {
            if binding.disk_reload != doc.disk_reload
                && !doc.dirty
                && !doc.media
                && binding.state.read(cx).as_str() == binding.synced
                && let Some(edit) =
                    crate::view_tabs_browser_model::minimal_edit(&binding.synced, &doc.text)
            {
                use gpui::EntityInputHandler;
                let from = binding.synced[..edit.start_byte].encode_utf16().count();
                let to = from
                    + binding.synced[edit.start_byte..edit.end_before_byte]
                        .encode_utf16()
                        .count();
                binding.state.update(cx, |state, cx| {
                    state.push_undo_stop();
                    state.replace_text_in_range(Some(from..to), &edit.replacement, window, cx);
                    state.push_undo_stop();
                });
                binding.synced.clone_from(&doc.text);
                binding.indentation = editor_indent::Options::detect(&doc.text);
            }
            binding.disk_reload = doc.disk_reload;
            if binding.state.read(cx).as_str() == doc.text {
                if restore_focus {
                    window.focus(&binding.state.read(cx).focus_handle(cx), cx);
                }
                return;
            }
        }
        // Rejected or unrelated model replacements retire the prior undo history.
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
                disk_reload: doc.disk_reload,
                _changed: changed,
                _caret: caret,
                indentation: editor_indent::Options::detect(&doc.text),
            },
        );
    }
}
