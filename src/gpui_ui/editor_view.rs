//! 1:1 `GPUI` Code & Unified Diff Editor, Rich Previews (size-gated Markdown + `gpui::canvas`
//! vector Mermaid `flowchart`/`sequenceDiagram`, sticky-header `SheetView`, JSON/JSONC/JSONL tree,
//! PDF, Office/OpenDocument, Images), and Windows WebView2 `Browser` tab surface.

use super::{
    FocusInputTarget, TesselGpuiRoot,
    icons::GpuiIcon,
    theme::{GpuiThemeTokens, mono_font_family, rgba_with_alpha},
};
use crate::{
    app::gpui_bridge::{GpuiAction, GpuiActiveDocumentSummary, GpuiWorkspaceSnapshot},
    rich_previews::{
        FlowDirection, FlowEdge, FlowNode, MarkdownSegment, MarkdownSizeDecision, MermaidDiagram,
        SequenceMessage, SequenceParticipant, classify_markdown_size, extract_markdown_segments,
        parse_mermaid_diagram,
    },
    syntax::{SyntaxPalette, tokenize_spans},
    theme_palettes::ThemeId,
};
use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, PathBuilder, Rgba,
    SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, img, point,
    prelude::FluentBuilder as _, px,
};

fn egui_color_to_gpui(c: eframe::egui::Color32) -> Rgba {
    gpui::rgb((u32::from(c.r()) << 16) | (u32::from(c.g()) << 8) | u32::from(c.b()))
}

// ---------------------------------------------------------------------------
// Deterministic Vector Mermaid Layout Geometry (shared by `gpui::canvas` & tests)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct MermaidPositionedNode {
    pub id: String,
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MermaidPositionedEdge {
    pub from_xy: (f32, f32),
    pub to_xy: (f32, f32),
    pub label: Option<String>,
    pub directed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MermaidFlowLayout {
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<MermaidPositionedNode>,
    pub edges: Vec<MermaidPositionedEdge>,
}

#[must_use]
pub fn layout_mermaid_flowchart(
    direction: FlowDirection,
    nodes: &[FlowNode],
    edges: &[FlowEdge],
) -> MermaidFlowLayout {
    let horizontal = matches!(
        direction,
        FlowDirection::LeftRight | FlowDirection::RightLeft
    );
    let reverse = matches!(
        direction,
        FlowDirection::BottomUp | FlowDirection::RightLeft
    );
    let node_w = 148.0_f32;
    let node_h = 38.0_f32;
    let gap_main = 54.0_f32;
    let pad = 20.0_f32;

    let count = nodes.len().max(1);
    let mut positioned = Vec::with_capacity(nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        let ord = if reverse { count - 1 - i } else { i } as f32;
        let (x, y) = if horizontal {
            (pad + ord * (node_w + gap_main), pad + 16.0)
        } else {
            (pad + 36.0, pad + ord * (node_h + gap_main))
        };
        positioned.push(MermaidPositionedNode {
            id: n.id.clone(),
            label: n.label.clone(),
            x,
            y,
            w: node_w,
            h: node_h,
        });
    }

    let mut pos_edges = Vec::with_capacity(edges.len());
    for e in edges {
        let Some(src) = positioned.iter().find(|p| p.id == e.from) else {
            continue;
        };
        let Some(dst) = positioned.iter().find(|p| p.id == e.to) else {
            continue;
        };
        let (from_xy, to_xy) = if horizontal {
            if src.x <= dst.x {
                (
                    (src.x + src.w, src.y + src.h * 0.5),
                    (dst.x, dst.y + dst.h * 0.5),
                )
            } else {
                (
                    (src.x, src.y + src.h * 0.5),
                    (dst.x + dst.w, dst.y + dst.h * 0.5),
                )
            }
        } else if src.y <= dst.y {
            (
                (src.x + src.w * 0.5, src.y + src.h),
                (dst.x + dst.w * 0.5, dst.y),
            )
        } else {
            (
                (src.x + src.w * 0.5, src.y),
                (dst.x + dst.w * 0.5, dst.y + dst.h),
            )
        };
        pos_edges.push(MermaidPositionedEdge {
            from_xy,
            to_xy,
            label: e.label.clone(),
            directed: e.directed,
        });
    }

    let width = positioned
        .iter()
        .map(|n| n.x + n.w + pad)
        .fold(240.0_f32, f32::max);
    let height = positioned
        .iter()
        .map(|n| n.y + n.h + pad)
        .fold(100.0_f32, f32::max);

    MermaidFlowLayout {
        width,
        height,
        nodes: positioned,
        edges: pos_edges,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MermaidSequenceLayout {
    pub width: f32,
    pub height: f32,
    pub actor_centers_x: Vec<(String, f32)>,
    pub message_rows: Vec<(f32, f32, f32, String, bool)>,
}

#[must_use]
pub fn layout_mermaid_sequence(
    participants: &[SequenceParticipant],
    messages: &[SequenceMessage],
) -> MermaidSequenceLayout {
    let col_w = 160.0_f32;
    let pad_x = 24.0_f32;
    let header_y = 54.0_f32;
    let row_step = 40.0_f32;

    let actor_centers_x: Vec<(String, f32)> = participants
        .iter()
        .enumerate()
        .map(|(i, p)| (p.label.clone(), pad_x + (i as f32) * col_w + 64.0))
        .collect();

    let find_x = |id: &str| -> f32 {
        participants
            .iter()
            .position(|p| p.id == id)
            .and_then(|idx| actor_centers_x.get(idx).map(|c| c.1))
            .unwrap_or(pad_x + 64.0)
    };

    let mut message_rows = Vec::with_capacity(messages.len());
    for (idx, m) in messages.iter().enumerate() {
        let y = header_y + 28.0 + (idx as f32) * row_step;
        message_rows.push((find_x(&m.from), find_x(&m.to), y, m.text.clone(), m.dashed));
    }

    let width = (pad_x * 2.0 + (participants.len().max(1) as f32) * col_w).max(260.0);
    let height = header_y + 44.0 + (messages.len() as f32) * row_step;

    MermaidSequenceLayout {
        width,
        height,
        actor_centers_x,
        message_rows,
    }
}

// ---------------------------------------------------------------------------
// Document & Diff Stage
// ---------------------------------------------------------------------------

pub(crate) fn render_document_editor_stage(
    snapshot: &GpuiWorkspaceSnapshot,
    editor_input: &super::editor_input::Input,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let Some(doc) = &snapshot.active_document else {
        return div().size_full().into_any_element();
    };
    let french = snapshot.french;

    let header = render_editor_toolbar(doc, french, tokens, cx).into_any_element();
    let tools_bar = render_editor_tools_bar(doc, &editor_input.tools, french, tokens, cx);

    let content = if let Some(state) = editor_input.plain() {
        gpui_ce_elements::editable_text::text_area("html-plain")
            .state(state.downgrade())
            .accepts_input(false)
            .debug_selector(|| "html-plain".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .p(px(20.))
            .font_family(mono_font_family())
            .line_height(px(21.))
            .text_color(tokens.text)
            .into_any_element()
    } else if let Some(source) = &doc.playback {
        render_playback(source, french, cx).into_any_element()
    } else if let Some(pdf) = &doc.pdf_summary {
        render_pdf_stage(pdf, PdfTarget::Local(doc.instance_id), french, tokens, cx)
            .into_any_element()
    } else if doc.office_summary.is_some() {
        render_office_stage(doc, french, tokens, cx).into_any_element()
    } else if let Some(dims) = doc.image_dims {
        render_image_stage(doc, dims, cx).into_any_element()
    } else if doc.preview_mode {
        if let Some(sheet) = &doc.sheet {
            super::sheet_view::render(sheet.clone(), french, tokens, cx).into_any_element()
        } else if let Some(json_prev) = &doc.json_preview {
            render_json_tree_stage(json_prev, french, tokens).into_any_element()
        } else if matches!(doc.extension.as_str(), "md" | "markdown") {
            render_markdown_stage(doc, french, tokens, cx).into_any_element()
        } else {
            render_readable_html_stage(doc, tokens, cx).into_any_element()
        }
    } else if let Some(native) = editor_input.render(doc, snapshot.font_size, tokens, cx) {
        native
    } else {
        render_code_or_diff_surface(
            doc,
            snapshot.theme,
            snapshot.dark,
            snapshot.font_size,
            tokens,
            cx,
        )
        .into_any_element()
    };

    div()
        .id("gpui-document-editor-stage")
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(tokens.term)
        .child(header)
        .when_some(tools_bar, |el, bar| el.child(bar))
        .when(!doc.error.is_empty(), |el| {
            el.child(
                div()
                    .px(px(12.0))
                    .py(px(4.0))
                    .bg(rgba_with_alpha(tokens.danger, 0.16))
                    .border_b_1()
                    .border_color(tokens.danger)
                    .text_size(px(11.5))
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(doc.error.clone())),
            )
        })
        .child(content)
        .into_any_element()
}

fn render_editor_toolbar(
    doc: &GpuiActiveDocumentSummary,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    div()
        .h(px(30.0))
        .px(px(12.0))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .bg(tokens.surface)
        .border_b_1()
        .border_color(tokens.border)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .overflow_hidden()
                .child(
                    if doc.is_diff {
                        GpuiIcon::FileDiff
                    } else {
                        GpuiIcon::FileCode
                    }
                    .render(13.0, tokens.accent),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(tokens.text)
                        .truncate()
                        .child(SharedString::from(doc.path_display.clone())),
                )
                .child(
                    div()
                        .px(px(5.0))
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .bg(tokens.surface_2)
                        .text_size(px(10.0))
                        .text_color(tokens.text_dim)
                        .child(doc.lang.display_name()),
                )
                .when(!doc.hunk_lines.is_empty(), |el| {
                    el.child(
                        div()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(tokens.accent_soft_bg)
                            .text_size(px(10.0))
                            .text_color(tokens.accent)
                            .child(SharedString::from(format!(
                                "{} modified",
                                doc.hunk_lines.len()
                            ))),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.0))
                .when(
                    doc.can_toggle_preview && matches!(doc.extension.as_str(), "htm" | "html"),
                    |mut el| {
                        use crate::app::gpui_bridge::HtmlViewMode::*;
                        let active = if doc.html_plain_text.is_some() {
                            Plain
                        } else if doc.preview_mode {
                            Preview
                        } else {
                            Source
                        };
                        for (mode, label, button) in [
                            (Source, "Source", "html-source-mode"),
                            (
                                Preview,
                                if french { "Aperçu" } else { "Preview" },
                                "html-preview-mode",
                            ),
                            (
                                Plain,
                                if french { "Texte brut" } else { "Plain text" },
                                "html-plain-mode",
                            ),
                        ] {
                            let id = doc.instance_id;
                            el = el.child(
                                div()
                                    .id(button)
                                    .debug_selector(move || button.into())
                                    .cursor_pointer()
                                    .rounded(px(4.))
                                    .px(px(7.))
                                    .py(px(2.5))
                                    .text_size(px(11.))
                                    .bg(if active == mode {
                                        tokens.accent_soft_bg
                                    } else {
                                        tokens.surface_2
                                    })
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(GpuiAction::SetHtmlView(id, mode), cx)
                                    }))
                                    .child(label),
                            );
                        }
                        #[cfg(windows)]
                        {
                            let id = doc.instance_id;
                            el = el.child(
                                div()
                                    .id("html-open-browser")
                                    .cursor_pointer()
                                    .px(px(7.))
                                    .py(px(2.5))
                                    .text_size(px(11.))
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(GpuiAction::OpenHtmlInBrowser(id), cx)
                                    }))
                                    .child(if french {
                                        "Ouvrir dans le navigateur"
                                    } else {
                                        "Open in browser"
                                    }),
                            );
                        }
                        el
                    },
                )
                .when(
                    doc.can_toggle_preview && !matches!(doc.extension.as_str(), "htm" | "html"),
                    |el| {
                        el.child(
                            div()
                                .id("editor-toggle-preview")
                                .px(px(8.0))
                                .py(px(2.5))
                                .rounded(px(4.0))
                                .bg(if doc.preview_mode {
                                    tokens.accent_soft_bg
                                } else {
                                    tokens.surface_2
                                })
                                .border_1()
                                .border_color(if doc.preview_mode {
                                    tokens.accent
                                } else {
                                    tokens.border
                                })
                                .text_size(px(11.0))
                                .text_color(tokens.text_strong)
                                .cursor_pointer()
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::ToggleActiveDocumentPreview, cx);
                                }))
                                .child(if doc.preview_mode {
                                    "Source"
                                } else if french {
                                    "Aperçu"
                                } else {
                                    "Preview"
                                }),
                        )
                    },
                )
                .when(!doc.media, |el| {
                    el.child(
                        div()
                            .id("editor-find-btn")
                            .px(px(7.0))
                            .py(px(2.5))
                            .rounded(px(4.0))
                            .bg(tokens.surface_2)
                            .text_size(px(11.0))
                            .text_color(tokens.text_dim)
                            .cursor_pointer()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.focus_input = FocusInputTarget::EditorFind;
                                root.dispatch(GpuiAction::ToggleEditorFind, cx);
                            }))
                            .child(if french { "Rechercher" } else { "Find" }),
                    )
                    .child(
                        div()
                            .id("editor-goto-btn")
                            .px(px(7.0))
                            .py(px(2.5))
                            .rounded(px(4.0))
                            .bg(tokens.surface_2)
                            .text_size(px(11.0))
                            .text_color(tokens.text_dim)
                            .cursor_pointer()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.focus_input = FocusInputTarget::EditorGoto;
                                root.dispatch(GpuiAction::ToggleEditorGoto, cx);
                            }))
                            .child(if french { "Ligne…" } else { "Go to line" }),
                    )
                    .child(
                        div()
                            .id("editor-save-btn")
                            .px(px(9.0))
                            .py(px(2.5))
                            .rounded(px(4.0))
                            .bg(if doc.dirty {
                                tokens.accent
                            } else {
                                tokens.surface_2
                            })
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if doc.dirty {
                                gpui::rgb(0xFF_FF_FF)
                            } else {
                                tokens.text_dim
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(GpuiAction::SaveActiveDocument, cx);
                            }))
                            .child(if french {
                                "Enregistrer"
                            } else {
                                "Save (Ctrl+S)"
                            }),
                    )
                }),
        )
}

fn render_editor_tools_bar(
    doc: &GpuiActiveDocumentSummary,
    inputs: &super::editor_tool_inputs::Inputs,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> Option<gpui::AnyElement> {
    if !doc.find_open && !doc.goto_open {
        return None;
    }
    let mut bar = div()
        .px(px(12.0))
        .py(px(5.0))
        .bg(tokens.chrome)
        .border_b_1()
        .border_color(tokens.border)
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.0))
        .text_size(px(11.5));

    let id = doc.instance_id;
    use super::editor_tool_inputs::{Command, run};
    use crate::app::gpui_bridge::EditorToolField;
    if doc.find_open {
        let badge = if doc.find_query.is_empty() {
            String::new()
        } else if doc.match_count == 0 {
            "0/0".into()
        } else {
            format!("{}/{}", doc.match_index + 1, doc.match_count)
        };
        bar = bar
            .child(inputs.render(doc, EditorToolField::Find, french, tokens, cx))
            .when(!badge.is_empty(), |el| {
                el.child(
                    div()
                        .text_color(tokens.text_dim)
                        .child(SharedString::from(badge)),
                )
            })
            .child(
                div()
                    .id("editor-find-prev")
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .bg(tokens.surface_2)
                    .cursor_pointer()
                    .on_click(cx.listener(move |root, _, window, cx| {
                        run(root, id, Command::Find(-1), window, cx)
                    }))
                    .child("↑"),
            )
            .child(
                div()
                    .id("editor-find-next")
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .bg(tokens.surface_2)
                    .cursor_pointer()
                    .on_click(cx.listener(move |root, _, window, cx| {
                        run(root, id, Command::Find(1), window, cx)
                    }))
                    .child("↓"),
            )
            .when(!doc.media && !doc.preview_mode, |el| {
                el.child(inputs.render(doc, EditorToolField::Replace, french, tokens, cx))
                    .child(
                        div()
                            .id("editor-replace-one")
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(tokens.surface_2)
                            .cursor_pointer()
                            .on_click(cx.listener(move |root, _, window, cx| {
                                run(root, id, Command::Replace(false), window, cx)
                            }))
                            .child(if french { "Remplacer" } else { "Replace" }),
                    )
                    .child(
                        div()
                            .id("editor-replace-all")
                            .px(px(7.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(tokens.surface_2)
                            .cursor_pointer()
                            .on_click(cx.listener(move |root, _, window, cx| {
                                run(root, id, Command::Replace(true), window, cx)
                            }))
                            .child(if french { "Tout" } else { "Replace all" }),
                    )
            });
    }

    if doc.goto_open {
        bar = bar
            .child(inputs.render(doc, EditorToolField::Goto, french, tokens, cx))
            .child(
                div()
                    .id("editor-goto-go")
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .bg(tokens.accent)
                    .text_color(gpui::rgb(0xFF_FF_FF))
                    .cursor_pointer()
                    .on_click(cx.listener(move |root, _, window, cx| {
                        run(root, id, Command::Goto, window, cx)
                    }))
                    .child("Go"),
            );
    }

    Some(bar.into_any_element())
}

fn render_code_or_diff_surface(
    doc: &GpuiActiveDocumentSummary,
    theme: ThemeId,
    dark: bool,
    font_size: f32,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let syntax_pal = SyntaxPalette::for_theme_mode(theme, dark);
    let line_h = (font_size * 1.42).round().max(15.0);

    let mut scroll = div()
        .id("gpui-code-editor-scroll")
        .flex_1()
        .min_h_0()
        .w_full()
        .py(px(8.0))
        .overflow_y_scroll()
        .font_family(mono_font_family())
        .text_size(px(font_size))
        .line_height(px(line_h))
        .bg(tokens.term)
        .on_click(cx.listener(|root, _, _, cx| {
            root.focus_input = FocusInputTarget::EditorBody;
            cx.notify();
        }));

    for (line_no, raw_line) in doc.preview_lines.iter().enumerate() {
        let is_modified = doc.hunk_lines.contains(&line_no);
        let is_match = doc.match_lines.contains(&line_no);

        let diff_bg = if doc.is_diff {
            if raw_line.starts_with('+') && !raw_line.starts_with("+++") {
                Some(rgba_with_alpha(tokens.ok_green, 0.14))
            } else if raw_line.starts_with('-') && !raw_line.starts_with("---") {
                Some(rgba_with_alpha(tokens.danger, 0.14))
            } else if raw_line.starts_with("@@") {
                Some(tokens.accent_soft_bg)
            } else {
                None
            }
        } else if is_match {
            Some(rgba_with_alpha(tokens.warn, 0.18))
        } else {
            None
        };

        let mut line_content = div().flex_1().flex().items_center().whitespace_nowrap();
        if doc.is_diff {
            let fg = if raw_line.starts_with('+') && !raw_line.starts_with("+++") {
                tokens.ok_green
            } else if raw_line.starts_with('-') && !raw_line.starts_with("---") {
                tokens.danger
            } else if raw_line.starts_with("@@") {
                tokens.accent
            } else {
                tokens.text
            };
            line_content = line_content.child(
                div()
                    .text_color(fg)
                    .child(SharedString::from(raw_line.clone())),
            );
        } else {
            let spans = tokenize_spans(raw_line, doc.lang);
            if spans.is_empty() {
                line_content = line_content.child(
                    div()
                        .text_color(egui_color_to_gpui(syntax_pal.plain))
                        .child(SharedString::from(raw_line.clone())),
                );
            } else {
                let mut cursor = 0usize;
                for (sb, eb, kind) in spans {
                    if sb > cursor {
                        line_content = line_content.child(
                            div()
                                .text_color(egui_color_to_gpui(syntax_pal.plain))
                                .child(SharedString::from(raw_line[cursor..sb].to_owned())),
                        );
                    }
                    if eb > sb {
                        let mut seg = div()
                            .text_color(egui_color_to_gpui(syntax_pal.color(kind)))
                            .child(SharedString::from(raw_line[sb..eb].to_owned()));
                        if kind == crate::syntax::TokenKind::Comment {
                            seg = seg.italic();
                        }
                        line_content = line_content.child(seg);
                        cursor = eb;
                    }
                }
                if cursor < raw_line.len() {
                    line_content = line_content.child(
                        div()
                            .text_color(egui_color_to_gpui(syntax_pal.plain))
                            .child(SharedString::from(raw_line[cursor..].to_owned())),
                    );
                }
            }
        }

        scroll = scroll.child(
            div()
                .w_full()
                .px(px(10.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .when_some(diff_bg, |el, bg| el.bg(bg))
                .child(
                    div()
                        .w(px(3.0))
                        .h(px(line_h))
                        .rounded(px(1.5))
                        .bg(if is_modified {
                            tokens.accent
                        } else {
                            gpui::rgba(0x0000_0000)
                        }),
                )
                .child(
                    div()
                        .w(px(40.0))
                        .text_right()
                        .text_color(tokens.text_dim)
                        .child(SharedString::from((line_no + 1).to_string())),
                )
                .child(line_content),
        );
    }

    scroll
}

// ---------------------------------------------------------------------------
// Rich Previews: Size-Gated Markdown + Vector Mermaid (`gpui::canvas`)
// ---------------------------------------------------------------------------

fn render_markdown_stage(
    doc: &GpuiActiveDocumentSummary,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let gate = classify_markdown_size(doc.text.len());
    match gate {
        MarkdownSizeDecision::SourceOnly { byte_len } => {
            return div()
                .p(px(18.0))
                .child(
                    div()
                        .p(px(14.0))
                        .rounded(px(tokens.radius_px))
                        .bg(tokens.surface)
                        .border_1()
                        .border_color(tokens.warn)
                        .text_size(px(12.0))
                        .text_color(tokens.text)
                        .child(SharedString::from(format!(
                            "Markdown file is {} KiB (> 1 MiB limit); use Source mode for instant editing.",
                            byte_len / 1024
                        ))),
                )
                .into_any_element();
        }
        MarkdownSizeDecision::PromptUser { byte_len } if !doc.force_large_markdown_render => {
            return div()
                .p(px(18.0))
                .child(
                    div()
                        .p(px(14.0))
                        .rounded(px(tokens.radius_px))
                        .bg(tokens.surface)
                        .border_1()
                        .border_color(tokens.accent)
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.0))
                        .child(div().text_size(px(12.0)).text_color(tokens.text).child(
                            SharedString::from(format!(
                                "Large Markdown document ({} KiB).",
                                byte_len / 1024
                            )),
                        ))
                        .child(
                            div()
                                .id("btn-force-md-render")
                                .px(px(10.0))
                                .py(px(4.5))
                                .rounded(px(tokens.radius_px))
                                .bg(tokens.accent)
                                .text_size(px(11.5))
                                .text_color(gpui::rgb(0xFF_FF_FF))
                                .cursor_pointer()
                                .on_click(cx.listener(|root, _, _, cx| {
                                    root.dispatch(GpuiAction::ForceRenderLargeMarkdown, cx);
                                }))
                                .child(if french {
                                    "Afficher quand même"
                                } else {
                                    "Render anyway"
                                }),
                        ),
                )
                .into_any_element();
        }
        _ => {}
    }

    let mut page = div()
        .id("gpui-markdown-preview")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(px(20.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(12.0));

    let mut md_link_idx = 0usize;
    for segment in extract_markdown_segments(&doc.text) {
        match segment {
            MarkdownSegment::Prose(prose) => {
                let parsed = crate::preview::Preview::parse(&prose, true);
                for (heading, spans) in parsed.blocks() {
                    let mut para = div().flex().flex_wrap().gap(px(4.0));
                    for (txt, href) in spans {
                        if let Some(link) = href {
                            let link_id = md_link_idx;
                            md_link_idx += 1;
                            para = para.child(
                                div()
                                    .id(("md-link", link_id))
                                    .text_size(px(if heading { 16.0 } else { 13.0 }))
                                    .font_weight(if heading {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(tokens.accent)
                                    .underline()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.dispatch(
                                            GpuiAction::FollowDocumentLink(link.clone()),
                                            cx,
                                        );
                                    }))
                                    .child(SharedString::from(txt)),
                            );
                        } else {
                            para = para.child(
                                div()
                                    .text_size(px(if heading { 16.0 } else { 13.0 }))
                                    .font_weight(if heading {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if heading {
                                        tokens.text_strong
                                    } else {
                                        tokens.text
                                    })
                                    .child(SharedString::from(txt)),
                            );
                        }
                    }
                    page = page.child(para);
                }
            }
            MarkdownSegment::Mermaid(src) => {
                page = page.child(render_mermaid_diagram_card(&src, tokens));
            }
        }
    }

    page.into_any_element()
}

fn render_mermaid_diagram_card(source: &str, tokens: &GpuiThemeTokens) -> gpui::AnyElement {
    match parse_mermaid_diagram(source) {
        Ok(MermaidDiagram::Flowchart {
            direction,
            nodes,
            edges,
        }) => {
            let geom = layout_mermaid_flowchart(direction, &nodes, &edges);
            let geom_paint = geom.clone();
            let accent = tokens.accent;

            let mut container = div()
                .relative()
                .w(px(geom.width))
                .h(px(geom.height))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, (), window, _cx| {
                            let ox = bounds.origin.x;
                            let oy = bounds.origin.y;
                            for edge in &geom_paint.edges {
                                let mut pb = PathBuilder::stroke(px(1.6));
                                let p0 = point(ox + px(edge.from_xy.0), oy + px(edge.from_xy.1));
                                let p1 = point(ox + px(edge.to_xy.0), oy + px(edge.to_xy.1));
                                pb.move_to(p0);
                                pb.line_to(p1);
                                if let Ok(path) = pb.build() {
                                    window.paint_path(path, accent);
                                }
                            }
                        },
                    )
                    .size_full(),
                );

            for node in &geom.nodes {
                container = container.child(
                    div()
                        .absolute()
                        .left(px(node.x))
                        .top(px(node.y))
                        .w(px(node.w))
                        .h(px(node.h))
                        .rounded(px(6.0))
                        .bg(tokens.surface_2)
                        .border_1()
                        .border_color(tokens.accent)
                        .flex()
                        .items_center()
                        .justify_center()
                        .px(px(6.0))
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(tokens.text_strong)
                        .truncate()
                        .child(SharedString::from(node.label.clone())),
                );
            }
            container.into_any_element()
        }
        Ok(MermaidDiagram::Sequence {
            participants,
            messages,
        }) => {
            let geom = layout_mermaid_sequence(&participants, &messages);
            let geom_paint = geom.clone();
            let accent = tokens.accent;
            let dim_line = tokens.border_strong;

            let mut container = div()
                .relative()
                .w(px(geom.width))
                .h(px(geom.height))
                .rounded(px(tokens.radius_px))
                .bg(tokens.surface)
                .border_1()
                .border_color(tokens.border)
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, (), window, _cx| {
                            let ox = bounds.origin.x;
                            let oy = bounds.origin.y;
                            for (_, cx_x) in &geom_paint.actor_centers_x {
                                let mut pb = PathBuilder::stroke(px(1.0));
                                pb.move_to(point(ox + px(*cx_x), oy + px(46.0)));
                                pb.line_to(point(
                                    ox + px(*cx_x),
                                    oy + px(geom_paint.height - 14.0),
                                ));
                                if let Ok(path) = pb.build() {
                                    window.paint_path(path, dim_line);
                                }
                            }
                            for (x0, x1, y, _, _) in &geom_paint.message_rows {
                                let mut pb = PathBuilder::stroke(px(1.6));
                                pb.move_to(point(ox + px(*x0), oy + px(*y)));
                                pb.line_to(point(ox + px(*x1), oy + px(*y)));
                                if let Ok(path) = pb.build() {
                                    window.paint_path(path, accent);
                                }
                            }
                        },
                    )
                    .size_full(),
                );

            for (label, cx_x) in &geom.actor_centers_x {
                container = container.child(
                    div()
                        .absolute()
                        .left(px(cx_x - 56.0))
                        .top(px(12.0))
                        .w(px(112.0))
                        .h(px(30.0))
                        .rounded(px(5.0))
                        .bg(tokens.surface_2)
                        .border_1()
                        .border_color(tokens.accent)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(tokens.text_strong)
                        .truncate()
                        .child(SharedString::from(label.clone())),
                );
            }
            for (x0, x1, y, msg, _) in &geom.message_rows {
                let left = x0.min(*x1);
                container = container.child(
                    div()
                        .absolute()
                        .left(px(left + 6.0))
                        .top(px(y - 17.0))
                        .text_size(px(10.5))
                        .text_color(tokens.text)
                        .child(SharedString::from(msg.clone())),
                );
            }
            container.into_any_element()
        }
        Err(_) => div()
            .p(px(10.0))
            .rounded(px(tokens.radius_px))
            .bg(tokens.surface)
            .border_1()
            .border_color(tokens.border)
            .font_family(mono_font_family())
            .text_size(px(11.5))
            .text_color(tokens.text_dim)
            .child(SharedString::from(source.to_owned()))
            .into_any_element(),
    }
}

fn render_readable_html_stage(
    doc: &GpuiActiveDocumentSummary,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let mut page = div()
        .id("gpui-html-preview")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(px(20.0))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(10.0));

    let mut html_link_idx = 0usize;
    for (heading, spans) in &doc.readable_blocks {
        let mut row = div().flex().flex_wrap().gap(px(4.0));
        for (txt, href) in spans {
            if let Some(link) = href.clone() {
                let context_link = link.clone();
                let document_id = doc.instance_id;
                let link_id = html_link_idx;
                html_link_idx += 1;
                row = row.child(
                    div()
                        .id(("html-link", link_id))
                        .debug_selector(move || format!("html-link-{link_id}"))
                        .on_mouse_down(
                            gpui::MouseButton::Right,
                            cx.listener(move |root, event: &gpui::MouseDownEvent, window, cx| {
                                window.prevent_default();
                                root.focus_handle.focus(window, cx);
                                cx.stop_propagation();
                                root.dispatch(
                                    GpuiAction::HtmlLinkMenu(
                                        document_id,
                                        context_link.clone(),
                                        (event.position.x.into(), event.position.y.into()),
                                    ),
                                    cx,
                                );
                            }),
                        )
                        .text_size(px(if *heading { 16.0 } else { 13.0 }))
                        .text_color(tokens.accent)
                        .underline()
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, _, cx| {
                            root.dispatch(GpuiAction::FollowDocumentLink(link.clone()), cx);
                        }))
                        .child(SharedString::from(txt.clone())),
                );
            } else {
                row = row.child(
                    div()
                        .text_size(px(if *heading { 16.0 } else { 13.0 }))
                        .font_weight(if *heading {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(tokens.text)
                        .child(SharedString::from(txt.clone())),
                );
            }
        }
        page = page.child(row);
    }
    page
}

// ---------------------------------------------------------------------------
// Rich Previews: Sticky-Header `SheetView` & JSON/JSONC/JSONL Tree
// ---------------------------------------------------------------------------

fn render_json_tree_stage(
    json_prev: &crate::rich_previews::JsonDocumentPreview,
    french: bool,
    tokens: &GpuiThemeTokens,
) -> impl IntoElement {
    let mut rows: Vec<(usize, String, String)> = Vec::new();
    flatten_json_preview(&json_prev.root, None, 0, &mut rows);

    let badge = if json_prev.is_jsonl {
        format!(
            "JSONL · {} {} ({} invalid)",
            json_prev.total_records,
            if french { "entrées" } else { "records" },
            json_prev.invalid_lines
        )
    } else {
        "JSON / JSONC Tree".into()
    };

    let mut list = div()
        .id("gpui-json-tree")
        .flex_1()
        .min_h_0()
        .p(px(12.0))
        .overflow_y_scroll()
        .font_family(mono_font_family())
        .text_size(px(12.0))
        .flex()
        .flex_col()
        .gap(px(3.0));

    for (depth, key, val) in rows {
        list = list.child(
            div()
                .pl(px((depth as f32) * 16.0))
                .flex()
                .items_center()
                .gap(px(6.0))
                .when(!key.is_empty(), |el| {
                    el.child(
                        div()
                            .text_color(tokens.accent)
                            .child(SharedString::from(format!("{key}:"))),
                    )
                })
                .child(div().text_color(tokens.text).child(SharedString::from(val))),
        );
    }

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .px(px(12.0))
                .py(px(5.0))
                .bg(tokens.chrome)
                .border_b_1()
                .border_color(tokens.border)
                .text_size(px(11.0))
                .text_color(tokens.text_dim)
                .child(SharedString::from(badge)),
        )
        .child(list)
}

fn flatten_json_preview(
    val: &serde_json::Value,
    key: Option<String>,
    depth: usize,
    out: &mut Vec<(usize, String, String)>,
) {
    if out.len() >= 600 || depth > 12 {
        return;
    }
    let k = key.unwrap_or_default();
    match val {
        serde_json::Value::Object(map) => {
            out.push((depth, k, format!("{{ {} keys }}", map.len())));
            for (ck, cv) in map.iter().take(120) {
                flatten_json_preview(cv, Some(ck.clone()), depth + 1, out);
            }
        }
        serde_json::Value::Array(arr) => {
            out.push((depth, k, format!("[ {} items ]", arr.len())));
            for (idx, cv) in arr.iter().take(120).enumerate() {
                flatten_json_preview(cv, Some(format!("[{idx}]")), depth + 1, out);
            }
        }
        serde_json::Value::String(s) => {
            let short: String = s.chars().take(160).collect();
            out.push((depth, k, format!("{short:?}")));
        }
        other => {
            out.push((depth, k, other.to_string()));
        }
    }
}

#[derive(Clone)]
pub(super) enum PdfTarget {
    Local(u64),
    Remote(String),
}

pub(super) fn render_pdf_stage(
    summary: &crate::pdf::PdfSummary,
    target: PdfTarget,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &Context<TesselGpuiRoot>,
) -> impl IntoElement {
    use crate::remote_panel::control::Command;
    let crate::pdf::PdfSummary {
        visible,
        pages,
        scale,
        busy,
        error,
        layout,
    } = summary;
    let button = |id: &'static str, label: &'static str, enabled: bool, action: GpuiAction| {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .px(px(8.))
            .py(px(2.5))
            .rounded(px(4.))
            .bg(tokens.surface_2)
            .opacity(if enabled { 1. } else { 0.45 })
            .when(enabled, |el| {
                el.cursor_pointer()
                    .on_click(cx.listener(move |root, _, _, cx| root.dispatch(action.clone(), cx)))
            })
            .child(label)
    };
    let zoom = |delta| match target {
        PdfTarget::Local(_) => GpuiAction::StepPdfZoom(delta),
        PdfTarget::Remote(_) => GpuiAction::Remote(Command::PdfZoom(delta)),
    };
    let fit = match &target {
        PdfTarget::Local(id) => GpuiAction::PdfFit(*id),
        PdfTarget::Remote(path) => GpuiAction::Remote(Command::PdfFit(path.clone())),
    };
    let toolbar = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(6.))
        .bg(tokens.chrome)
        .border_b_1()
        .border_color(tokens.border)
        .when(*pages > 0, |el| {
            el.child(SharedString::from(format!(
                "{pages} {}",
                if *pages == 1 { "page" } else { "pages" }
            )))
        })
        .child(div().flex_1())
        .child(button("pdf-zoom-out", "−", *scale > 0.25, zoom(-0.25)))
        .child(SharedString::from(format!("{:.0} %", scale * 100.)))
        .child(button("pdf-zoom-in", "+", *scale < 4., zoom(0.25)))
        .child(button(
            "pdf-fit",
            if french {
                "Ajuster à la largeur"
            } else {
                "Fit width"
            },
            !layout.fit,
            fit,
        ));
    let owner = cx.weak_entity();
    let previous = layout.viewport;
    let stage_id = match &target {
        PdfTarget::Local(id) => format!("local-pdf-scroll-{id}"),
        PdfTarget::Remote(path) => format!("remote-pdf-scroll-{path}"),
    };
    let probe = canvas(
        |_, _, _| (),
        move |bounds, (), window, cx| {
            let clip = window.content_mask().bounds;
            let viewport = eframe::egui::Rect::from_min_size(
                eframe::egui::pos2(
                    f32::from(clip.origin.x - bounds.origin.x).round(),
                    f32::from(clip.origin.y - bounds.origin.y).round(),
                ),
                eframe::egui::vec2(
                    f32::from(clip.size.width).round(),
                    f32::from(clip.size.height).round(),
                ),
            );
            if viewport.is_positive() && viewport != previous {
                let target = target.clone();
                let owner = owner.clone();
                cx.defer(move |cx| {
                    let _ = owner.update(cx, |root, cx| {
                        let changed = match &target {
                            PdfTarget::Local(id) => root.app.set_local_pdf_viewport(*id, viewport),
                            PdfTarget::Remote(path) => {
                                root.app.set_remote_pdf_viewport(path, viewport)
                            }
                        };
                        if changed {
                            cx.notify();
                        }
                    });
                });
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full();
    let mut content = div()
        .relative()
        .flex_none()
        .w(px(layout.extent.x))
        .h(px(layout.extent.y))
        .min_w_full()
        .min_h_full();
    for page in visible {
        let index = page.index;
        let rect = page.rect;
        let placeholder = div()
            .absolute()
            .left(px(rect.left()))
            .top(px(rect.top()))
            .w(px(rect.width()))
            .h(px(rect.height()))
            .bg(gpui::white());
        content = content.child(placeholder.when_some(page.pixels.clone(), |el, pixels| {
            el.child(
                img(pixels)
                    .id(("pdf-page-image", index))
                    .debug_selector(move || format!("pdf-page-image-{index}"))
                    .size_full(),
            )
        }));
    }
    let stage = div()
        .id(SharedString::from(stage_id))
        .debug_selector(|| "pdf-page-viewport".into())
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_scroll()
        .child(content.child(probe));
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(toolbar)
        .when(!error.is_empty(), |el| {
            el.child(SharedString::from(error.clone()))
        })
        .when(*pages == 0 && *busy, |el| {
            el.child(if french {
                "Chargement…"
            } else {
                "Loading…"
            })
        })
        .child(stage)
}

fn render_office_stage(
    doc: &GpuiActiveDocumentSummary,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let summary = doc.office_summary.clone().unwrap_or_default();
    render_office_summary(&summary, french, tokens, cx, false)
}

pub(crate) fn render_office_summary(
    summary: &crate::office::OfficeSummary,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &Context<TesselGpuiRoot>,
    remote: bool,
) -> gpui::Div {
    let mut tabs = div()
        .px(px(10.0))
        .py(px(5.0))
        .bg(tokens.chrome)
        .border_b_1()
        .border_color(tokens.border)
        .flex()
        .items_center()
        .gap(px(6.0));

    for (idx, title) in summary.section_titles.iter().enumerate() {
        let active = idx == summary.selected;
        tabs = tabs.child(
            div()
                .id(("office-sec", idx))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(4.0))
                .bg(if active {
                    tokens.accent_soft_bg
                } else {
                    tokens.surface_2
                })
                .border_1()
                .border_color(if active { tokens.accent } else { tokens.border })
                .text_size(px(11.0))
                .text_color(tokens.text_strong)
                .cursor_pointer()
                .on_click(cx.listener(move |root, _, _, cx| {
                    root.dispatch(
                        if remote {
                            GpuiAction::Remote(
                                crate::remote_panel::control::Command::OfficeSection(idx),
                            )
                        } else {
                            GpuiAction::SelectOfficeSection(idx)
                        },
                        cx,
                    );
                }))
                .child(SharedString::from(title.clone())),
        );
    }

    let body = if let Some(sheet) = &summary.sheet {
        super::sheet_view::render(sheet.clone(), french, tokens, cx).into_any_element()
    } else {
        let mut p_col = div()
            .id("office-paragraphs")
            .flex_1()
            .min_h_0()
            .p(px(18.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.0));
        for (heading, text) in &summary.paragraphs {
            p_col = p_col.child(
                div()
                    .text_size(px(if *heading { 16.0 } else { 13.0 }))
                    .font_weight(if *heading {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(if *heading {
                        tokens.text_strong
                    } else {
                        tokens.text
                    })
                    .child(SharedString::from(text.clone())),
            );
        }
        p_col.into_any_element()
    };

    div().size_full().flex().flex_col()
        .child(if french {
            "Aperçu en lecture seule · macros et formules non exécutées · mise en forme approximative"
        } else {
            "Read-only preview · macros and formulas are not executed · formatting may differ"
        })
        .when(summary.busy, |el| el.child(if french { "Chargement du document…" } else { "Loading document…" }))
        .when(!summary.error.is_empty(), |el| el.child(SharedString::from(summary.error.clone())))
        .when(summary.truncated, |el| el.child(if french { "Aperçu tronqué aux limites du document." } else { "Preview truncated at the document limits." }))
        .child(tabs).child(body)
}

pub(super) fn render_playback(
    source: &crate::files::playback::Source,
    french: bool,
    cx: &Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let stage = div()
        .id("media-player-viewport")
        .debug_selector(|| "media-player-viewport".into())
        .flex_1()
        .min_h_0()
        .w_full()
        .relative();
    #[cfg(windows)]
    {
        let owner = cx.weak_entity();
        let source = source.clone();
        stage
            .when(source.startup_failed(), |el| {
                el.child(crate::files::playback::startup_error(french))
            })
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, (), window, cx| {
                        let bounds = bounds.intersect(&window.content_mask().bounds);
                        let rect = eframe::egui::Rect::from_min_size(
                            eframe::egui::pos2(bounds.origin.x.into(), bounds.origin.y.into()),
                            eframe::egui::vec2(bounds.size.width.into(), bounds.size.height.into()),
                        );
                        let _ = owner.update(cx, |root, cx| {
                            if root.app.gpui_media_unobscured() {
                                let before = source.startup_failed();
                                root.app.media_player.sync(
                                    window,
                                    &source,
                                    rect,
                                    window.scale_factor(),
                                    french,
                                );
                                if before != source.startup_failed() {
                                    cx.notify();
                                }
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
    }
    #[cfg(not(windows))]
    {
        let _ = (source, cx);
        stage.child(if french {
            "La lecture multimédia intégrée nécessite Windows."
        } else {
            "Embedded media playback requires Windows."
        })
    }
}

/// Keep the image and its right-click target at the same fitted size.
pub(super) fn preview_image_pixels(image: std::sync::Arc<gpui::RenderImage>) -> gpui::Div {
    use gpui::StyledImage;
    div().relative().overflow_hidden().child(
        img(image)
            .absolute()
            .size_full()
            .object_fit(gpui::ObjectFit::Contain),
    )
}

pub(super) fn fitted_preview_image(
    image: impl Styled + IntoElement + 'static,
    [width, height]: [usize; 2],
) -> impl IntoElement {
    canvas(
        move |bounds, window, cx| {
            let fitted = crate::files::fitted_image_size(
                eframe::egui::vec2(width as f32, height as f32),
                eframe::egui::vec2(f32::from(bounds.size.width), f32::from(bounds.size.height)),
            );
            if fitted.x <= 0. || fitted.y <= 0. {
                return None;
            }
            let size = gpui::size(px(fitted.x), px(fitted.y));
            let origin = bounds.center() - point(size.width / 2., size.height / 2.);
            let mut element = image
                .w(size.width)
                .h(size.height)
                .max_w(size.width)
                .max_h(size.height)
                .into_any_element();
            element.prepaint_as_root(origin, size.map(gpui::AvailableSpace::Definite), window, cx);
            Some(element)
        },
        |_, element, window, cx| {
            if let Some(mut element) = element {
                element.paint(window, cx);
            }
        },
    )
    .flex_1()
    .min_h_0()
    .w_full()
}

fn render_image_stage(
    doc: &GpuiActiveDocumentSummary,
    dims: [usize; 2],
    cx: &Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let id = doc.instance_id;
    let mut stage = div()
        .debug_selector(|| "local-image-viewport".into())
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .p(px(20.));
    if let Some(image) = &doc.media_pixels {
        stage = stage.child(fitted_preview_image(
            preview_image_pixels(image.clone())
                .id("local-preview-image")
                .debug_selector(|| "local-preview-image".into())
                .on_mouse_down(
                    gpui::MouseButton::Right,
                    cx.listener(move |root, event: &gpui::MouseDownEvent, window, cx| {
                        window.prevent_default();
                        root.focus_handle.focus(window, cx);
                        cx.stop_propagation();
                        root.dispatch(
                            GpuiAction::ImageMenu(
                                id,
                                Some((event.position.x.into(), event.position.y.into())),
                            ),
                            cx,
                        );
                    }),
                ),
            dims,
        ));
    }
    if !doc.image_download_notice.is_empty() {
        stage = stage.child(SharedString::from(doc.image_download_notice.clone()));
    }
    stage
}

// ---------------------------------------------------------------------------
// Embedded Browser Tab (`#[cfg(windows)]` WebView2 Child HWND Sync)
// ---------------------------------------------------------------------------

pub(crate) fn render_browser_stage(
    snapshot: &GpuiWorkspaceSnapshot,
    browser_input: &super::browser_input::Input,
    tokens: &GpuiThemeTokens,
    _window: &mut Window,
    cx: &mut Context<TesselGpuiRoot>,
) -> impl IntoElement {
    let french = snapshot.french;
    let b = &snapshot.browser;
    let expected_address_input = browser_input.state().cloned();
    let can_show = snapshot.overlay_title.is_none() && !snapshot.floating.open;

    #[cfg(windows)]
    let view_entity = cx.entity().downgrade();

    let viewport_body = if !b.started {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .p(px(24.0))
            .child(
                div()
                    .max_w(px(460.0))
                    .text_size(px(12.5))
                    .text_color(tokens.text_dim)
                    .child(if french {
                        "Profil de navigateur isolé pour Tessel Rust. Aucun cookie ni mot de passe externe n'est importé automatiquement."
                    } else {
                        "Private browser profile for Tessel Rust. No external cookies or passwords are imported automatically."
                    }),
            )
            .child(
                div()
                    .id("gpui-browser-start")
                    .px(px(14.0))
                    .py(px(7.0))
                    .rounded(px(tokens.radius_px))
                    .bg(tokens.accent)
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(gpui::rgb(0xFF_FF_FF))
                    .cursor_pointer()
                    .on_click(cx.listener(|root, _, window, cx| {
                        #[cfg(windows)]
                        root.app.browser.start_for_parent(window);
                        #[cfg(not(windows))]
                        let _ = (root, window);
                        cx.notify();
                    }))
                    .child(if french {
                        "Démarrer le navigateur"
                    } else {
                        "Start browser"
                    }),
            )
            .into_any_element()
    } else {
        #[cfg(windows)]
        {
            canvas(
                |_, _, _| (),
                move |bounds, (), window, cx| {
                    let scale = window.scale_factor();
                    let x = f32::from(bounds.origin.x);
                    let y = f32::from(bounds.origin.y);
                    let w = f32::from(bounds.size.width);
                    let h = f32::from(bounds.size.height);
                    let _ = view_entity.update(cx, |root, cx| {
                        if root
                            .app
                            .browser
                            .sync_gpui_bounds(x, y, w, h, scale, can_show, None, !can_show)
                        {
                            root.app.browser.cancel_native_address_edit();
                            root.focus_input = FocusInputTarget::None;
                            window.blur(cx);
                        }
                    });
                },
            )
            .size_full()
            .into_any_element()
        }
        #[cfg(not(windows))]
        {
            let _ = can_show;
            div().flex_1().into_any_element()
        }
    };

    div()
        .id("gpui-browser-stage")
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(tokens.term)
        // Navigation bar
        .child(
            div()
                .h(px(34.0))
                .px(px(8.0))
                .bg(tokens.surface)
                .border_b_1()
                .border_color(tokens.border)
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .id("browser-back")
                        .px(px(7.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .bg(tokens.surface_2)
                        .cursor_pointer()
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.dispatch(GpuiAction::BrowserBack, cx);
                        }))
                        .child("←"),
                )
                .child(
                    div()
                        .id("browser-fwd")
                        .px(px(7.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .bg(tokens.surface_2)
                        .cursor_pointer()
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.dispatch(GpuiAction::BrowserForward, cx);
                        }))
                        .child("→"),
                )
                .child(
                    div()
                        .id("browser-reload")
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .bg(tokens.surface_2)
                        .text_size(px(11.0))
                        .cursor_pointer()
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.dispatch(GpuiAction::BrowserReload, cx);
                        }))
                        .child(if french { "Actualiser" } else { "Reload" }),
                )
                .child(browser_input.render(french, tokens, cx))
                .child(
                    div()
                        .id("browser-go")
                        .px(px(9.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .bg(tokens.accent)
                        .text_size(px(11.0))
                        .text_color(gpui::rgb(0xFF_FF_FF))
                        .cursor_pointer()
                        .on_click(cx.listener(move |root, _, window, cx| {
                            if expected_address_input.as_ref() == root.browser_input.state() {
                                super::browser_input::navigate(root, window, cx);
                            }
                        }))
                        .child("Go"),
                )
                .child(
                    div()
                        .id("browser-downloads-btn")
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .bg(tokens.surface_2)
                        .text_size(px(11.0))
                        .text_color(tokens.text)
                        .cursor_pointer()
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.dispatch(GpuiAction::BrowserToggleDownloads, cx);
                        }))
                        .child(SharedString::from(format!(
                            "{} ({})",
                            if french {
                                "Téléchargements"
                            } else {
                                "Downloads"
                            },
                            b.downloads.len()
                        ))),
                ),
        )
        .child(
            div()
                .px(px(10.0))
                .py(px(4.0))
                .flex()
                .items_center()
                .gap(px(10.0))
                .child(
                    div()
                        .id("browser-import-cookies")
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(4.0))
                        .bg(tokens.surface_2)
                        .text_size(px(11.0))
                        .cursor_pointer()
                        .on_click(cx.listener(|root, _, _, cx| {
                            root.dispatch(GpuiAction::BrowserImportCookies, cx);
                        }))
                        .child(if french {
                            "Importer un fichier de cookies…"
                        } else {
                            "Import cookies from file…"
                        }),
                )
                .child(SharedString::from(b.cookie_import_status.clone())),
        )
        .when_some(b.popup_url.as_ref(), |el, _popup| {
            el.child(
                div()
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(tokens.surface_2)
                    .border_b_1()
                    .border_color(tokens.accent)
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(11.5))
                    .child(if french {
                        "La page a demandé une nouvelle fenêtre."
                    } else {
                        "Page requested a new window."
                    })
                    .child(
                        div()
                            .id("browser-popup-accept")
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(tokens.accent)
                            .text_color(gpui::rgb(0xFF_FF_FF))
                            .cursor_pointer()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(GpuiAction::BrowserAcceptPopup, cx);
                            }))
                            .child(if french {
                                "Ouvrir dans cet onglet"
                            } else {
                                "Open in this tab"
                            }),
                    )
                    .child(
                        div()
                            .id("browser-popup-dismiss")
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded(px(4.0))
                            .bg(tokens.surface_3)
                            .cursor_pointer()
                            .on_click(cx.listener(|root, _, _, cx| {
                                root.dispatch(GpuiAction::BrowserDismissPopup, cx);
                            }))
                            .child(if french { "Fermer" } else { "Dismiss" }),
                    ),
            )
        })
        .when(!b.error.is_empty(), |el| {
            el.child(
                div()
                    .px(px(10.0))
                    .py(px(4.0))
                    .bg(rgba_with_alpha(tokens.danger, 0.16))
                    .border_b_1()
                    .border_color(tokens.danger)
                    .text_size(px(11.0))
                    .text_color(tokens.text_strong)
                    .child(SharedString::from(b.error.clone())),
            )
        })
        .child(viewport_body)
}
