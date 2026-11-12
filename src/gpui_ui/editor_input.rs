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
    indentation: editor_indent::Options,
}

gpui::actions!(tessel_local_editor, [Indent, Outdent]);

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
                state.set_selected_text_range(8..0, window, cx)
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
        self.bindings.insert(
            id,
            Binding {
                state,
                synced: doc.text.clone(),
                _changed: changed,
                indentation: editor_indent::Options::detect(&doc.text),
            },
        );
    }
}
