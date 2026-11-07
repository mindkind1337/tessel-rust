//! Rich native document previews (`SheetView.vue`, `MarkdownView.vue`):
//! - Sticky-header `SheetTable` grid (`MAX_SHEET_ROWS = 5000`, `MAX_SHEET_COLS = 200`, `A..ZZ` columns,
//!   1-based row numbers, cell inspector bar, row filter) + RFC 4180 CSV/TSV parser.
//! - Collapsible JSON / JSONC / JSONL / NDJSON tree viewer (`strip_jsonc_comments`, `invalid_lines` count).
//! - `MarkdownSizeGate` (`<= 600 KiB` auto-render, `600 KiB..=1 MiB` user prompt, `> 1 MiB` source-only).
//! - Native vector Mermaid diagram parser & `egui::Painter` renderer (`flowchart`/`graph` & `sequenceDiagram`)
//!   with clean monospace code-block fallback for unsupported diagram types.

use std::collections::{BTreeMap, HashMap, VecDeque};

use eframe::egui;
use serde_json::Value;

pub const MAX_SHEET_ROWS: usize = 5_000;
pub const MAX_SHEET_COLS: usize = 200;
pub const MAX_JSONL_RECORDS: usize = 2_000;
pub const MARKDOWN_AUTO_RENDER_MAX: usize = 600 * 1024;
pub const MARKDOWN_PROMPT_MAX: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// 1. Markdown Size Gate & Mermaid Segment Splitter
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownSizeDecision {
    AutoRender,
    PromptUser { byte_len: usize },
    SourceOnly { byte_len: usize },
}

#[must_use]
pub fn classify_markdown_size(byte_len: usize) -> MarkdownSizeDecision {
    if byte_len <= MARKDOWN_AUTO_RENDER_MAX {
        MarkdownSizeDecision::AutoRender
    } else if byte_len <= MARKDOWN_PROMPT_MAX {
        MarkdownSizeDecision::PromptUser { byte_len }
    } else {
        MarkdownSizeDecision::SourceOnly { byte_len }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownSegment {
    Prose(String),
    Mermaid(String),
}

/// Splits Markdown text into alternating prose chunks and ```mermaid fenced blocks.
#[must_use]
pub fn extract_markdown_segments(source: &str) -> Vec<MarkdownSegment> {
    let mut segments = Vec::new();
    let mut prose_buf = String::new();
    let mut mermaid_buf = String::new();
    let mut fence_marker: Option<&str> = None;
    let mut in_other_fence: Option<&str> = None;

    for raw_line in source.split_inclusive('\n') {
        let trimmed = raw_line.trim_start();
        if let Some(active_fence) = fence_marker {
            if trimmed.starts_with(active_fence) && trimmed[active_fence.len()..].trim().is_empty()
            {
                segments.push(MarkdownSegment::Mermaid(
                    mermaid_buf.trim_end_matches(['\r', '\n']).to_owned(),
                ));
                mermaid_buf.clear();
                fence_marker = None;
            } else {
                mermaid_buf.push_str(raw_line);
            }
            continue;
        }
        if let Some(other) = in_other_fence {
            prose_buf.push_str(raw_line);
            if trimmed.starts_with(other) && trimmed[other.len()..].trim().is_empty() {
                in_other_fence = None;
            }
            continue;
        }
        let fence_candidate = if trimmed.starts_with("```") {
            Some("```")
        } else if trimmed.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        if let Some(marker) = fence_candidate {
            let lang = trimmed[marker.len()..].trim();
            if lang.eq_ignore_ascii_case("mermaid") {
                if !prose_buf.trim().is_empty() {
                    segments.push(MarkdownSegment::Prose(std::mem::take(&mut prose_buf)));
                } else {
                    prose_buf.clear();
                }
                fence_marker = Some(marker);
                continue;
            }
            in_other_fence = Some(marker);
        }
        prose_buf.push_str(raw_line);
    }

    if fence_marker.is_some() && !mermaid_buf.is_empty() {
        // Unclosed fence: treat as prose code block so nothing is lost.
        prose_buf.push_str("```mermaid\n");
        prose_buf.push_str(&mermaid_buf);
    }
    if !prose_buf.trim().is_empty() {
        segments.push(MarkdownSegment::Prose(prose_buf));
    }
    segments
}

// ---------------------------------------------------------------------------
// 2. Native Vector Mermaid Parser & Painter (`flowchart` + `sequenceDiagram`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowDirection {
    TopDown,
    BottomUp,
    LeftRight,
    RightLeft,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowNodeShape {
    Rect,
    Rounded,
    Stadium,
    Diamond,
    Circle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowNode {
    pub id: String,
    pub label: String,
    pub shape: FlowNodeShape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowEdgeStyle {
    Solid,
    Dotted,
    Thick,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
    pub style: FlowEdgeStyle,
    pub directed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceParticipant {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceMessage {
    pub from: String,
    pub to: String,
    pub text: String,
    pub dashed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MermaidDiagram {
    Flowchart {
        direction: FlowDirection,
        nodes: Vec<FlowNode>,
        edges: Vec<FlowEdge>,
    },
    Sequence {
        participants: Vec<SequenceParticipant>,
        messages: Vec<SequenceMessage>,
    },
}

pub fn parse_mermaid_diagram(source: &str) -> Result<MermaidDiagram, String> {
    let mut lines = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let header = lines.next().ok_or("Empty Mermaid block")?;
    let mut header_tokens = header.split_whitespace();
    let kind = header_tokens.next().unwrap_or("");

    if kind.eq_ignore_ascii_case("flowchart") || kind.eq_ignore_ascii_case("graph") {
        let dir = match header_tokens.next().unwrap_or("TD") {
            d if d.eq_ignore_ascii_case("LR") => FlowDirection::LeftRight,
            d if d.eq_ignore_ascii_case("RL") => FlowDirection::RightLeft,
            d if d.eq_ignore_ascii_case("BT") => FlowDirection::BottomUp,
            _ => FlowDirection::TopDown,
        };
        return parse_flowchart_body(dir, lines);
    }
    if kind.eq_ignore_ascii_case("sequenceDiagram") {
        return parse_sequence_body(lines);
    }
    Err(format!("Unsupported Mermaid diagram type: {kind}"))
}

fn parse_flowchart_body<'a>(
    direction: FlowDirection,
    lines: impl Iterator<Item = &'a str>,
) -> Result<MermaidDiagram, String> {
    let mut node_order: Vec<String> = Vec::new();
    let mut node_map: HashMap<String, FlowNode> = HashMap::new();
    let mut edges: Vec<FlowEdge> = Vec::new();

    let upsert_node = |raw_spec: &str,
                       order: &mut Vec<String>,
                       map: &mut HashMap<String, FlowNode>|
     -> Option<String> {
        let parsed = parse_flow_node_spec(raw_spec)?;
        let id = parsed.id.clone();
        if let Some(existing) = map.get_mut(&id) {
            if parsed.label != parsed.id || parsed.shape != FlowNodeShape::Rect {
                existing.label = parsed.label;
                existing.shape = parsed.shape;
            }
        } else {
            order.push(id.clone());
            map.insert(id.clone(), parsed);
        }
        Some(id)
    };

    for raw_line in lines {
        let line = raw_line.trim_end_matches(';').trim();
        if line.is_empty()
            || line.starts_with("classDef ")
            || line.starts_with("class ")
            || line.starts_with("style ")
            || line.starts_with("linkStyle ")
            || line.starts_with("click ")
            || line.starts_with("direction ")
            || line.eq_ignore_ascii_case("end")
        {
            continue;
        }
        if let Some(rest) = line.strip_prefix("subgraph ") {
            let _ = rest;
            continue;
        }
        if let Some((lhs, edge_style, directed, edge_label, rhs)) = split_flow_edge(line) {
            let Some(from_id) = upsert_node(lhs, &mut node_order, &mut node_map) else {
                continue;
            };
            // Support chained edges `A --> B --> C`
            let mut prev_id = from_id;
            let mut cur_style = edge_style;
            let mut cur_dir = directed;
            let mut cur_lbl = edge_label;
            let mut rem = rhs;
            loop {
                if let Some((mid, next_style, next_dir, next_lbl, tail)) = split_flow_edge(rem) {
                    if let Some(mid_id) = upsert_node(mid, &mut node_order, &mut node_map) {
                        edges.push(FlowEdge {
                            from: prev_id,
                            to: mid_id.clone(),
                            label: cur_lbl,
                            style: cur_style,
                            directed: cur_dir,
                        });
                        prev_id = mid_id;
                    }
                    cur_style = next_style;
                    cur_dir = next_dir;
                    cur_lbl = next_lbl;
                    rem = tail;
                } else {
                    if let Some(to_id) = upsert_node(rem, &mut node_order, &mut node_map) {
                        edges.push(FlowEdge {
                            from: prev_id,
                            to: to_id,
                            label: cur_lbl,
                            style: cur_style,
                            directed: cur_dir,
                        });
                    }
                    break;
                }
            }
        } else {
            let _ = upsert_node(line, &mut node_order, &mut node_map);
        }
    }

    if node_order.is_empty() {
        return Err("Flowchart contains no nodes".into());
    }
    let nodes = node_order
        .into_iter()
        .filter_map(|id| node_map.remove(&id))
        .collect();
    Ok(MermaidDiagram::Flowchart {
        direction,
        nodes,
        edges,
    })
}

fn clean_mermaid_label(raw: &str) -> String {
    let t = raw.trim();
    let unquoted = t
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| t.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(t);
    unquoted.replace("<br/>", "\n").replace("<br>", "\n")
}

fn parse_flow_node_spec(raw: &str) -> Option<FlowNode> {
    let trimmed = raw.trim();
    let trimmed = trimmed.split(":::").next().unwrap_or(trimmed).trim();
    if trimmed.is_empty() {
        return None;
    }
    let id_end = trimmed.find(['[', '(', '{']).unwrap_or(trimmed.len());
    let id = trimmed[..id_end].trim();
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return None;
    }
    let rest = trimmed[id_end..].trim();
    if rest.is_empty() {
        return Some(FlowNode {
            id: id.to_owned(),
            label: id.to_owned(),
            shape: FlowNodeShape::Rect,
        });
    }
    let (shape, inner) = if let Some(s) = rest.strip_prefix("([").and_then(|s| s.strip_suffix("])"))
    {
        (FlowNodeShape::Stadium, s)
    } else if let Some(s) = rest.strip_prefix("((").and_then(|s| s.strip_suffix("))")) {
        (FlowNodeShape::Circle, s)
    } else if let Some(s) = rest.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        (FlowNodeShape::Rect, s)
    } else if let Some(s) = rest.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        (FlowNodeShape::Rounded, s)
    } else if let Some(s) = rest.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
        (FlowNodeShape::Diamond, s)
    } else {
        (FlowNodeShape::Rect, id)
    };
    let label = clean_mermaid_label(inner);
    Some(FlowNode {
        id: id.to_owned(),
        label: if label.is_empty() {
            id.to_owned()
        } else {
            label
        },
        shape,
    })
}

fn split_flow_edge(line: &str) -> Option<(&str, FlowEdgeStyle, bool, Option<String>, &str)> {
    // Check operators outside brackets/quotes:
    let bytes = line.as_bytes();
    let mut depth = 0_i32;
    let mut in_quotes = false;
    let mut i = 0_usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quotes = !in_quotes,
            b'[' | b'(' | b'{' if !in_quotes => depth += 1,
            b']' | b')' | b'}' if !in_quotes => depth = (depth - 1).max(0),
            _ if !in_quotes && depth == 0 => {
                let tail = &line[i..];
                for (pat, style, directed) in [
                    ("-.->", FlowEdgeStyle::Dotted, true),
                    ("-.-", FlowEdgeStyle::Dotted, false),
                    ("==>", FlowEdgeStyle::Thick, true),
                    ("===", FlowEdgeStyle::Thick, false),
                    ("-->", FlowEdgeStyle::Solid, true),
                    ("---", FlowEdgeStyle::Solid, false),
                ] {
                    if let Some(after_op) = tail.strip_prefix(pat) {
                        let lhs = line[..i].trim();
                        if lhs.is_empty() {
                            return None;
                        }
                        let after_trimmed = after_op.trim_start();
                        if let Some(pipe_rest) = after_trimmed.strip_prefix('|')
                            && let Some(end_pipe) = pipe_rest.find('|')
                        {
                            let lbl = clean_mermaid_label(&pipe_rest[..end_pipe]);
                            let rhs = pipe_rest[end_pipe + 1..].trim();
                            return Some((
                                lhs,
                                style,
                                directed,
                                (!lbl.is_empty()).then_some(lbl),
                                rhs,
                            ));
                        }
                        return Some((lhs, style, directed, None, after_trimmed));
                    }
                }
                // Also support `-- Label -->` inline edge syntax:
                if let Some(after_dash) = tail.strip_prefix("-- ")
                    && let Some(arrow_rel) = after_dash.find("-->")
                {
                    let lhs = line[..i].trim();
                    let lbl = clean_mermaid_label(&after_dash[..arrow_rel]);
                    let rhs = after_dash[arrow_rel + 3..].trim();
                    if !lhs.is_empty() && !rhs.is_empty() {
                        return Some((
                            lhs,
                            FlowEdgeStyle::Solid,
                            true,
                            (!lbl.is_empty()).then_some(lbl),
                            rhs,
                        ));
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_sequence_body<'a>(lines: impl Iterator<Item = &'a str>) -> Result<MermaidDiagram, String> {
    let mut participants: Vec<SequenceParticipant> = Vec::new();
    let mut known: HashMap<String, usize> = HashMap::new();
    let mut messages: Vec<SequenceMessage> = Vec::new();

    let ensure_participant =
        |raw: &str, list: &mut Vec<SequenceParticipant>, map: &mut HashMap<String, usize>| {
            let id = raw.trim().to_owned();
            if !id.is_empty() && !map.contains_key(&id) {
                map.insert(id.clone(), list.len());
                list.push(SequenceParticipant {
                    label: id.clone(),
                    id,
                });
            }
        };

    for raw_line in lines {
        let line = raw_line.trim();
        if let Some(rest) = line
            .strip_prefix("participant ")
            .or_else(|| line.strip_prefix("actor "))
        {
            let (id_part, label_part) = rest
                .split_once(" as ")
                .map_or((rest.trim(), rest.trim()), |(i, l)| (i.trim(), l.trim()));
            if !id_part.is_empty() {
                let lbl = clean_mermaid_label(label_part);
                if let Some(&idx) = known.get(id_part) {
                    participants[idx].label = lbl;
                } else {
                    known.insert(id_part.to_owned(), participants.len());
                    participants.push(SequenceParticipant {
                        id: id_part.to_owned(),
                        label: lbl,
                    });
                }
            }
            continue;
        }
        for (arrow, dashed) in [("-->>", true), ("-->", true), ("->>", false), ("->", false)] {
            if let Some((left, right)) = line.split_once(arrow)
                && let Some((target_raw, msg_raw)) = right.split_once(':')
            {
                let from = left.trim().trim_end_matches(['+', '-']).trim();
                let to = target_raw.trim().trim_start_matches(['+', '-']).trim();
                if !from.is_empty() && !to.is_empty() {
                    ensure_participant(from, &mut participants, &mut known);
                    ensure_participant(to, &mut participants, &mut known);
                    messages.push(SequenceMessage {
                        from: from.to_owned(),
                        to: to.to_owned(),
                        text: clean_mermaid_label(msg_raw),
                        dashed,
                    });
                    break;
                }
            }
        }
    }
    if participants.is_empty() {
        return Err("Sequence diagram contains no participants".into());
    }
    Ok(MermaidDiagram::Sequence {
        participants,
        messages,
    })
}

pub fn show_mermaid_block(ui: &mut egui::Ui, source: &str) {
    let palette = crate::theme::active(ui.ctx()).spec().ui;
    egui::Frame::NONE
        .fill(palette.surface)
        .stroke(egui::Stroke::new(1.0_f32, palette.border))
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| match parse_mermaid_diagram(source) {
            Ok(diagram) => {
                egui::ScrollArea::horizontal()
                    .id_salt(("mermaid-canvas", source.len()))
                    .show(ui, |ui| {
                        paint_mermaid_diagram(ui, &diagram, &palette);
                    });
            }
            Err(_) => {
                ui.small(
                    egui::RichText::new("Mermaid diagram (source fallback)")
                        .color(palette.text_dim),
                );
                ui.add_space(4.0_f32);
                ui.label(
                    egui::RichText::new(source)
                        .monospace()
                        .size(12.0_f32)
                        .color(palette.text),
                );
            }
        });
}

pub fn paint_mermaid_diagram(
    ui: &mut egui::Ui,
    diagram: &MermaidDiagram,
    palette: &crate::theme_palettes::UiPalette,
) {
    match diagram {
        MermaidDiagram::Flowchart {
            direction,
            nodes,
            edges,
        } => paint_flowchart(ui, *direction, nodes, edges, palette),
        MermaidDiagram::Sequence {
            participants,
            messages,
        } => paint_sequence(ui, participants, messages, palette),
    }
}

fn paint_flowchart(
    ui: &mut egui::Ui,
    direction: FlowDirection,
    nodes: &[FlowNode],
    edges: &[FlowEdge],
    palette: &crate::theme_palettes::UiPalette,
) {
    // Compute BFS rank per node for clean layered DAG placement:
    let mut indegree: HashMap<&str, usize> = nodes.iter().map(|n| (n.id.as_str(), 0)).collect();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in edges {
        if e.from != e.to {
            adj.entry(e.from.as_str()).or_default().push(e.to.as_str());
            if let Some(d) = indegree.get_mut(e.to.as_str()) {
                *d += 1;
            }
        }
    }
    let mut rank: HashMap<&str, usize> = HashMap::new();
    let mut queue: VecDeque<&str> = nodes
        .iter()
        .filter(|n| indegree.get(n.id.as_str()).copied().unwrap_or(0) == 0)
        .map(|n| n.id.as_str())
        .collect();
    if queue.is_empty()
        && let Some(first) = nodes.first()
    {
        queue.push_back(first.id.as_str());
    }
    while let Some(u) = queue.pop_front() {
        let r = rank.get(u).copied().unwrap_or(0);
        rank.entry(u).or_insert(r);
        if let Some(nexts) = adj.get(u) {
            for &v in nexts {
                let next_r = r + 1;
                if rank
                    .get(v)
                    .is_none_or(|&cur| cur < next_r && next_r <= nodes.len())
                {
                    rank.insert(v, next_r);
                }
                if let Some(ind) = indegree.get_mut(v)
                    && *ind > 0
                {
                    *ind -= 1;
                    if *ind == 0 {
                        queue.push_back(v);
                    }
                }
            }
        }
    }
    let mut layers: BTreeMap<usize, Vec<&FlowNode>> = BTreeMap::new();
    for node in nodes {
        let r = rank.get(node.id.as_str()).copied().unwrap_or(0);
        layers.entry(r).or_default().push(node);
    }
    let max_layer_idx = layers.keys().copied().max().unwrap_or(0);
    let max_breadth = layers.values().map(Vec::len).max().unwrap_or(1);

    let horizontal = matches!(
        direction,
        FlowDirection::LeftRight | FlowDirection::RightLeft
    );
    let reversed = matches!(
        direction,
        FlowDirection::BottomUp | FlowDirection::RightLeft
    );
    let node_w = 148.0_f32;
    let node_h = 44.0_f32;
    let gap_primary = 72.0_f32;
    let gap_cross = 28.0_f32;
    let pad = 18.0_f32;

    let canvas_w = if horizontal {
        pad * 2.0_f32 + (max_layer_idx + 1) as f32 * node_w + max_layer_idx as f32 * gap_primary
    } else {
        pad * 2.0_f32
            + max_breadth as f32 * node_w
            + max_breadth.saturating_sub(1) as f32 * gap_cross
    };
    let canvas_h = if horizontal {
        pad * 2.0_f32
            + max_breadth as f32 * node_h
            + max_breadth.saturating_sub(1) as f32 * gap_cross
    } else {
        pad * 2.0_f32 + (max_layer_idx + 1) as f32 * node_h + max_layer_idx as f32 * gap_primary
    };

    let (rect, _) = ui.allocate_exact_size(egui::vec2(canvas_w, canvas_h), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let mut centers: HashMap<&str, egui::Rect> = HashMap::new();
    for (&layer_idx, group) in &layers {
        let eff_layer = if reversed {
            max_layer_idx.saturating_sub(layer_idx)
        } else {
            layer_idx
        };
        let count = group.len();
        for (slot, node) in group.iter().enumerate() {
            let nr = if horizontal {
                let total_h = count as f32 * node_h + count.saturating_sub(1) as f32 * gap_cross;
                let y0 = rect.min.y
                    + (canvas_h - total_h) * 0.5_f32
                    + slot as f32 * (node_h + gap_cross);
                let x0 = rect.min.x + pad + eff_layer as f32 * (node_w + gap_primary);
                egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(node_w, node_h))
            } else {
                let total_w = count as f32 * node_w + count.saturating_sub(1) as f32 * gap_cross;
                let x0 = rect.min.x
                    + (canvas_w - total_w) * 0.5_f32
                    + slot as f32 * (node_w + gap_cross);
                let y0 = rect.min.y + pad + eff_layer as f32 * (node_h + gap_primary);
                egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(node_w, node_h))
            };
            centers.insert(node.id.as_str(), nr);
        }
    }

    // Paint edges first so nodes sit cleanly above connectors:
    for edge in edges {
        let (Some(&r_from), Some(&r_to)) = (
            centers.get(edge.from.as_str()),
            centers.get(edge.to.as_str()),
        ) else {
            continue;
        };
        let (p1, p2) = if horizontal {
            if r_from.center().x <= r_to.center().x {
                (
                    egui::pos2(r_from.max.x, r_from.center().y),
                    egui::pos2(r_to.min.x, r_to.center().y),
                )
            } else {
                (
                    egui::pos2(r_from.min.x, r_from.center().y),
                    egui::pos2(r_to.max.x, r_to.center().y),
                )
            }
        } else if r_from.center().y <= r_to.center().y {
            (
                egui::pos2(r_from.center().x, r_from.max.y),
                egui::pos2(r_to.center().x, r_to.min.y),
            )
        } else {
            (
                egui::pos2(r_from.center().x, r_from.min.y),
                egui::pos2(r_to.center().x, r_to.max.y),
            )
        };
        let stroke_w = if edge.style == FlowEdgeStyle::Thick {
            2.2_f32
        } else {
            1.4_f32
        };
        let stroke = egui::Stroke::new(stroke_w, palette.border_strong);
        if edge.style == FlowEdgeStyle::Dotted {
            paint_dashed_segment(&painter, p1, p2, stroke);
        } else {
            painter.line_segment([p1, p2], stroke);
        }
        if edge.directed {
            paint_arrowhead(&painter, p1, p2, palette.accent);
        }
        if let Some(lbl) = &edge.label {
            let mid = p1 + (p2 - p1) * 0.5_f32;
            let galley = painter.layout_no_wrap(
                lbl.clone(),
                egui::FontId::proportional(11.0_f32),
                palette.text,
            );
            let pill =
                egui::Rect::from_center_size(mid, galley.size() + egui::vec2(10.0_f32, 4.0_f32));
            painter.rect_filled(pill, egui::CornerRadius::same(4), palette.chrome);
            painter.rect_stroke(
                pill,
                egui::CornerRadius::same(4),
                egui::Stroke::new(1.0_f32, palette.border),
                egui::StrokeKind::Inside,
            );
            painter.galley(
                pill.center() - galley.size() * 0.5_f32,
                galley,
                palette.text,
            );
        }
    }

    // Paint nodes:
    for node in nodes {
        let Some(&nr) = centers.get(node.id.as_str()) else {
            continue;
        };
        let stroke = egui::Stroke::new(1.25_f32, palette.accent);
        match node.shape {
            FlowNodeShape::Diamond => {
                let pts = vec![
                    egui::pos2(nr.center().x, nr.min.y),
                    egui::pos2(nr.max.x, nr.center().y),
                    egui::pos2(nr.center().x, nr.max.y),
                    egui::pos2(nr.min.x, nr.center().y),
                ];
                painter.add(egui::Shape::convex_polygon(pts, palette.surface_2, stroke));
            }
            FlowNodeShape::Circle => {
                let radius = nr.height().min(nr.width()) * 0.5_f32;
                painter.circle(nr.center(), radius, palette.surface_2, stroke);
            }
            FlowNodeShape::Stadium => {
                let cr = egui::CornerRadius::same(20);
                painter.rect_filled(nr, cr, palette.surface_2);
                painter.rect_stroke(nr, cr, stroke, egui::StrokeKind::Inside);
            }
            FlowNodeShape::Rounded => {
                let cr = egui::CornerRadius::same(10);
                painter.rect_filled(nr, cr, palette.surface_2);
                painter.rect_stroke(nr, cr, stroke, egui::StrokeKind::Inside);
            }
            FlowNodeShape::Rect => {
                let cr = egui::CornerRadius::same(5);
                painter.rect_filled(nr, cr, palette.surface_2);
                painter.rect_stroke(nr, cr, stroke, egui::StrokeKind::Inside);
            }
        }
        let galley = painter.layout(
            node.label.clone(),
            egui::FontId::proportional(12.0_f32),
            palette.text_strong,
            nr.width() - 14.0_f32,
        );
        painter.galley(
            nr.center() - galley.size() * 0.5_f32,
            galley,
            palette.text_strong,
        );
    }
}

fn paint_sequence(
    ui: &mut egui::Ui,
    participants: &[SequenceParticipant],
    messages: &[SequenceMessage],
    palette: &crate::theme_palettes::UiPalette,
) {
    let col_w = 160.0_f32;
    let box_w = 124.0_f32;
    let box_h = 34.0_f32;
    let row_h = 44.0_f32;
    let pad = 18.0_f32;
    let width = pad * 2.0_f32 + participants.len() as f32 * col_w;
    let height = pad * 2.0_f32 + box_h + 24.0_f32 + messages.len().max(1) as f32 * row_h;

    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter_at(rect);

    let mut x_by_id: HashMap<&str, f32> = HashMap::new();
    for (idx, part) in participants.iter().enumerate() {
        let cx = rect.min.x + pad + idx as f32 * col_w + col_w * 0.5_f32;
        x_by_id.insert(part.id.as_str(), cx);
        let box_rect = egui::Rect::from_center_size(
            egui::pos2(cx, rect.min.y + pad + box_h * 0.5_f32),
            egui::vec2(box_w, box_h),
        );
        let cr = egui::CornerRadius::same(6);
        painter.rect_filled(box_rect, cr, palette.surface_2);
        painter.rect_stroke(
            box_rect,
            cr,
            egui::Stroke::new(1.2_f32, palette.accent),
            egui::StrokeKind::Inside,
        );
        let galley = painter.layout_no_wrap(
            part.label.clone(),
            egui::FontId::proportional(12.0_f32),
            palette.text_strong,
        );
        painter.galley(
            box_rect.center() - galley.size() * 0.5_f32,
            galley,
            palette.text_strong,
        );
        paint_dashed_segment(
            &painter,
            egui::pos2(cx, box_rect.max.y + 4.0_f32),
            egui::pos2(cx, rect.max.y - pad),
            egui::Stroke::new(1.0_f32, palette.border_strong),
        );
    }

    for (m_idx, msg) in messages.iter().enumerate() {
        let (Some(&x1), Some(&x2)) = (x_by_id.get(msg.from.as_str()), x_by_id.get(msg.to.as_str()))
        else {
            continue;
        };
        let y = rect.min.y + pad + box_h + 28.0_f32 + m_idx as f32 * row_h;
        let p1 = egui::pos2(x1, y);
        let p2 = if (x2 - x1).abs() < 1.0_f32 {
            egui::pos2(x1 + 36.0_f32, y)
        } else {
            egui::pos2(x2, y)
        };
        let stroke = egui::Stroke::new(1.4_f32, palette.accent);
        if msg.dashed {
            paint_dashed_segment(&painter, p1, p2, stroke);
        } else {
            painter.line_segment([p1, p2], stroke);
        }
        paint_arrowhead(&painter, p1, p2, palette.accent);
        if !msg.text.is_empty() {
            let mid = egui::pos2((p1.x + p2.x) * 0.5_f32, y - 10.0_f32);
            let galley = painter.layout_no_wrap(
                msg.text.clone(),
                egui::FontId::proportional(11.0_f32),
                palette.text,
            );
            painter.galley(mid - galley.size() * 0.5_f32, galley, palette.text);
        }
    }
}

fn paint_dashed_segment(
    painter: &egui::Painter,
    p1: egui::Pos2,
    p2: egui::Pos2,
    stroke: egui::Stroke,
) {
    let delta = p2 - p1;
    let total = delta.length();
    if total <= 0.5_f32 {
        return;
    }
    let dir = delta / total;
    let dash = 6.0_f32;
    let gap = 4.0_f32;
    let mut t = 0.0_f32;
    while t < total {
        let t_end = (t + dash).min(total);
        painter.line_segment([p1 + dir * t, p1 + dir * t_end], stroke);
        t += dash + gap;
    }
}

fn paint_arrowhead(
    painter: &egui::Painter,
    from: egui::Pos2,
    to: egui::Pos2,
    color: egui::Color32,
) {
    let delta = to - from;
    let len = delta.length();
    if len < 2.0_f32 {
        return;
    }
    let dir = delta / len;
    let perp = egui::vec2(-dir.y, dir.x);
    let tip = to;
    let base = to - dir * 8.0_f32;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, base + perp * 4.0_f32, base - perp * 4.0_f32],
        color,
        egui::Stroke::NONE,
    ));
}

// ---------------------------------------------------------------------------
// 3. `SheetTable` & RFC 4180 CSV/TSV Grid (`SheetView.vue`)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SheetTable {
    pub rows: Vec<Vec<String>>,
    pub start_row: usize,
    pub start_col: usize,
    pub truncated_rows: usize,
    pub truncated_cols: usize,
    pub delimiter_label: &'static str,
}

#[must_use]
pub fn sheet_col_letter(mut col: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push((b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.into_iter().rev().collect()
}

#[must_use]
pub fn detect_csv_delimiter(source: &str) -> (u8, &'static str) {
    let sample: Vec<&str> = source
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(10)
        .collect();
    if sample.is_empty() {
        return (b',', "CSV (,)");
    }
    let count_outside_quotes = |line: &str, delim: u8| -> usize {
        let mut in_q = false;
        let mut c = 0usize;
        for &b in line.as_bytes() {
            if b == b'"' {
                in_q = !in_q;
            } else if !in_q && b == delim {
                c += 1;
            }
        }
        c
    };
    let mut best = (b',', "CSV (,)", 0usize);
    for (delim, label) in [(b'\t', "TSV (tab)"), (b',', "CSV (,)"), (b';', "CSV (;)")] {
        let first = count_outside_quotes(sample[0], delim);
        if first > 0 {
            let consistent = sample
                .iter()
                .filter(|l| count_outside_quotes(l, delim) == first)
                .count();
            let score = consistent * 100 + first;
            if score > best.2 {
                best = (delim, label, score);
            }
        }
    }
    (best.0, best.1)
}

#[must_use]
pub fn parse_delimited_sheet(raw: &str, explicit_delim: Option<u8>) -> SheetTable {
    let text = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let (delim, delimiter_label) = match explicit_delim {
        Some(b'\t') => (b'\t', "TSV (tab)"),
        Some(b';') => (b';', "CSV (;)"),
        Some(b',') => (b',', "CSV (,)"),
        Some(other) => (other, "Delimited"),
        None => detect_csv_delimiter(text),
    };

    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut cur_row: Vec<String> = Vec::new();
    let mut cur_cell = String::new();
    let mut in_quotes = false;
    let mut truncated_rows = 0usize;
    let mut truncated_cols = 0usize;

    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    let _ = chars.next();
                    cur_cell.push('"');
                } else {
                    in_quotes = false;
                }
            } else {
                cur_cell.push(ch);
            }
            continue;
        }
        if ch == '"' && cur_cell.is_empty() {
            in_quotes = true;
        } else if ch.len_utf8() == 1 && ch as u8 == delim {
            if cur_row.len() < MAX_SHEET_COLS {
                cur_row.push(std::mem::take(&mut cur_cell));
            } else {
                truncated_cols += 1;
                cur_cell.clear();
            }
        } else if ch == '\r' || ch == '\n' {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            if cur_row.len() < MAX_SHEET_COLS {
                cur_row.push(std::mem::take(&mut cur_cell));
            } else {
                truncated_cols += 1;
                cur_cell.clear();
            }
            if rows.len() < MAX_SHEET_ROWS {
                rows.push(std::mem::take(&mut cur_row));
            } else {
                truncated_rows += 1;
                cur_row.clear();
            }
        } else {
            cur_cell.push(ch);
        }
    }
    if !cur_cell.is_empty() || !cur_row.is_empty() {
        if cur_row.len() < MAX_SHEET_COLS {
            cur_row.push(cur_cell);
        } else {
            truncated_cols += 1;
        }
        if rows.len() < MAX_SHEET_ROWS {
            rows.push(cur_row);
        } else {
            truncated_rows += 1;
        }
    }

    SheetTable {
        rows,
        start_row: 0,
        start_col: 0,
        truncated_rows,
        truncated_cols,
        delimiter_label,
    }
}

pub fn show_sheet_grid(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + Copy,
    sheet: &SheetTable,
) {
    let palette = crate::theme::active(ui.ctx()).spec().ui;
    let french = crate::i18n::is_french();
    let state_id = ui.make_persistent_id((id_salt, "sheet-grid-state"));
    let (mut selected, mut filter): (Option<(usize, usize)>, String) = ui
        .ctx()
        .data_mut(|d| d.get_temp(state_id).unwrap_or((None, String::new())));

    let total_cols = sheet
        .rows
        .iter()
        .map(Vec::len)
        .max()
        .unwrap_or(0)
        .min(MAX_SHEET_COLS);

    ui.horizontal(|ui| {
        ui.small(
            egui::RichText::new(format!(
                "{} · {}×{}",
                sheet.delimiter_label,
                sheet.rows.len(),
                total_cols
            ))
            .color(palette.text_dim),
        );
        if sheet.truncated_rows > 0 || sheet.truncated_cols > 0 {
            ui.colored_label(
                palette.warn,
                if french {
                    format!("Aperçu limité à {MAX_SHEET_ROWS} lignes × {MAX_SHEET_COLS} colonnes")
                } else {
                    format!("Preview capped at {MAX_SHEET_ROWS} rows × {MAX_SHEET_COLS} columns")
                },
            );
        }
        ui.separator();
        ui.add(
            egui::TextEdit::singleline(&mut filter)
                .hint_text(if french {
                    "Filtrer les lignes…"
                } else {
                    "Filter rows…"
                })
                .desired_width(180.0_f32),
        );
        if let Some((r, c)) = selected
            && let Some(val) = sheet.rows.get(r).and_then(|row| row.get(c))
        {
            let addr = format!(
                "{}{}",
                sheet_col_letter(sheet.start_col + c),
                sheet.start_row + r + 1
            );
            ui.separator();
            ui.label(egui::RichText::new(addr).monospace().strong());
            ui.add(egui::Label::new(egui::RichText::new(val).monospace()).truncate());
        }
    });
    ui.add_space(4.0_f32);

    let needle = filter.trim().to_lowercase();
    let visible_indices: Vec<usize> = if needle.is_empty() {
        (0..sheet.rows.len()).collect()
    } else {
        sheet
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.iter().any(|cell| cell.to_lowercase().contains(&needle)))
            .map(|(idx, _)| idx)
            .collect()
    };

    egui::ScrollArea::horizontal()
        .id_salt((id_salt, "sheet-hscroll"))
        .show(ui, |ui| {
            // Sticky header row:
            egui::Frame::NONE
                .fill(palette.surface_2)
                .stroke(egui::Stroke::new(1.0_f32, palette.border))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0_f32;
                        ui.add_sized(
                            [54.0_f32, 24.0_f32],
                            egui::Label::new(
                                egui::RichText::new("#").monospace().color(palette.text_dim),
                            ),
                        );
                        for col in 0..total_cols {
                            ui.add_sized(
                                [132.0_f32, 24.0_f32],
                                egui::Label::new(
                                    egui::RichText::new(sheet_col_letter(sheet.start_col + col))
                                        .monospace()
                                        .strong()
                                        .color(palette.text_strong),
                                )
                                .truncate(),
                            );
                        }
                    });
                });

            egui::ScrollArea::vertical()
                .id_salt((id_salt, "sheet-vscroll"))
                .show_rows(ui, 23.0_f32, visible_indices.len(), |ui, range| {
                    for vis_pos in range {
                        let r_idx = visible_indices[vis_pos];
                        let row = &sheet.rows[r_idx];
                        let row_bg = if vis_pos % 2 == 0 {
                            palette.surface
                        } else {
                            palette.chrome
                        };
                        egui::Frame::NONE.fill(row_bg).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0_f32;
                                ui.add_sized(
                                    [54.0_f32, 22.0_f32],
                                    egui::Label::new(
                                        egui::RichText::new(
                                            (sheet.start_row + r_idx + 1).to_string(),
                                        )
                                        .monospace()
                                        .color(palette.text_dim),
                                    ),
                                );
                                for c_idx in 0..total_cols {
                                    let cell = row.get(c_idx).map_or("", String::as_str);
                                    let is_sel = selected == Some((r_idx, c_idx));
                                    let resp = ui.add_sized(
                                        [132.0_f32, 22.0_f32],
                                        egui::Button::new(
                                            egui::RichText::new(cell)
                                                .size(12.0_f32)
                                                .color(palette.text),
                                        )
                                        .fill(if is_sel {
                                            palette.surface_3
                                        } else {
                                            egui::Color32::TRANSPARENT
                                        })
                                        .stroke(if is_sel {
                                            egui::Stroke::new(1.0_f32, palette.accent)
                                        } else {
                                            egui::Stroke::new(0.5_f32, palette.border)
                                        })
                                        .truncate(),
                                    );
                                    if resp.clicked() {
                                        selected = Some((r_idx, c_idx));
                                    }
                                    if !cell.is_empty() {
                                        resp.on_hover_text(cell);
                                    }
                                }
                            });
                        });
                    }
                });
        });

    ui.ctx()
        .data_mut(|d| d.insert_temp(state_id, (selected, filter)));
}

// ---------------------------------------------------------------------------
// 4. Collapsible JSON / JSONC / JSONL / NDJSON Tree Viewer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct JsonDocumentPreview {
    pub root: Value,
    pub is_jsonl: bool,
    pub total_records: usize,
    pub invalid_lines: usize,
}

/// Strips `//` line comments, `/* ... */` block comments, and trailing commas outside strings
/// so `.jsonc` files (such as `tsconfig.json` / `.vscode/settings.json`) parse cleanly.
#[must_use]
pub fn strip_jsonc_comments(source: &str) -> String {
    let text = source.strip_prefix('\u{feff}').unwrap_or(source);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_str = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_str {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        if ch == '"' {
            in_str = true;
            out.push(ch);
        } else if ch == '/' && chars.peek() == Some(&'/') {
            let _ = chars.next();
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if ch == '/' && chars.peek() == Some(&'*') {
            let _ = chars.next();
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'/') {
                    let _ = chars.next();
                    break;
                }
                if c == '\n' {
                    out.push('\n');
                }
            }
        } else {
            out.push(ch);
        }
    }
    remove_jsonc_trailing_commas(&out)
}

fn remove_jsonc_trailing_commas(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut in_str = false;
    let mut escaped = false;
    let mut idx = 0usize;
    while idx < bytes.len() {
        let ch = input[idx..].chars().next().expect("char boundary");
        let next_idx = idx + ch.len_utf8();
        if in_str {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_str = false;
            }
            idx = next_idx;
            continue;
        }
        if ch == '"' {
            in_str = true;
            out.push(ch);
        } else if ch == ',' {
            let rest = input[next_idx..].trim_start();
            if !rest.starts_with('}') && !rest.starts_with(']') {
                out.push(',');
            }
        } else {
            out.push(ch);
        }
        idx = next_idx;
    }
    out
}

pub fn parse_json_or_jsonl(source: &str, extension: &str) -> Result<JsonDocumentPreview, String> {
    if matches!(extension, "jsonl" | "ndjson") {
        let mut items = Vec::new();
        let mut invalid_lines = 0usize;
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if items.len() >= MAX_JSONL_RECORDS {
                break;
            }
            match serde_json::from_str::<Value>(trimmed) {
                Ok(v) => items.push(v),
                Err(_) => invalid_lines += 1,
            }
        }
        let total_records = items.len();
        return Ok(JsonDocumentPreview {
            root: Value::Array(items),
            is_jsonl: true,
            total_records,
            invalid_lines,
        });
    }
    let cleaned = if extension == "jsonc" {
        strip_jsonc_comments(source)
    } else {
        source.strip_prefix('\u{feff}').unwrap_or(source).to_owned()
    };
    let root: Value = serde_json::from_str(&cleaned)
        .or_else(|_| serde_json::from_str(&strip_jsonc_comments(source)))
        .map_err(|e| format!("Invalid JSON: {e}"))?;
    let total_records = match &root {
        Value::Array(a) => a.len(),
        Value::Object(m) => m.len(),
        _ => 1,
    };
    Ok(JsonDocumentPreview {
        root,
        is_jsonl: false,
        total_records,
        invalid_lines: 0,
    })
}

pub fn show_json_preview(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + Copy,
    source: &str,
    extension: &str,
) {
    let palette = crate::theme::active(ui.ctx()).spec().ui;
    let french = crate::i18n::is_french();
    let filter_id = ui.make_persistent_id((id_salt, "json-filter"));
    let mut filter: String = ui
        .ctx()
        .data_mut(|d| d.get_temp(filter_id).unwrap_or_default());

    match parse_json_or_jsonl(source, extension) {
        Ok(doc) => {
            ui.horizontal(|ui| {
                ui.small(
                    egui::RichText::new(if doc.is_jsonl {
                        format!("JSONL · {} records", doc.total_records)
                    } else {
                        format!("JSON · {} top-level entries", doc.total_records)
                    })
                    .color(palette.text_dim),
                );
                if doc.invalid_lines > 0 {
                    ui.colored_label(
                        palette.warn,
                        format!("{} invalid line(s) skipped", doc.invalid_lines),
                    );
                }
                ui.separator();
                ui.add(
                    egui::TextEdit::singleline(&mut filter)
                        .hint_text(if french {
                            "Filtrer les clés…"
                        } else {
                            "Filter keys…"
                        })
                        .desired_width(180.0_f32),
                );
                if ui
                    .button(if french {
                        "Copier le JSON formaté"
                    } else {
                        "Copy formatted JSON"
                    })
                    .clicked()
                    && let Ok(pretty) = serde_json::to_string_pretty(&doc.root)
                {
                    ui.ctx().copy_text(pretty);
                }
            });
            ui.add_space(6.0_f32);
            let needle = filter.trim().to_lowercase();
            egui::ScrollArea::vertical()
                .id_salt((id_salt, "json-tree-scroll"))
                .show(ui, |ui| {
                    show_json_node(ui, "root", &doc.root, 0, &needle, &palette);
                });
        }
        Err(err) => {
            ui.colored_label(palette.danger, err);
        }
    }

    ui.ctx().data_mut(|d| d.insert_temp(filter_id, filter));
}

fn show_json_node(
    ui: &mut egui::Ui,
    key: &str,
    val: &Value,
    depth: usize,
    needle: &str,
    palette: &crate::theme_palettes::UiPalette,
) {
    if depth > 24 {
        ui.small("… (depth limit)");
        return;
    }
    match val {
        Value::Object(map) => {
            let header = format!("{key} {{{}}}", map.len());
            egui::CollapsingHeader::new(
                egui::RichText::new(header)
                    .monospace()
                    .color(palette.text_strong),
            )
            .id_salt((key, depth))
            .default_open(depth < 2)
            .show(ui, |ui| {
                for (k, child) in map {
                    if !needle.is_empty() && depth >= 1 && !json_subtree_matches(k, child, needle) {
                        continue;
                    }
                    show_json_node(ui, k, child, depth + 1, needle, palette);
                }
            });
        }
        Value::Array(arr) => {
            let header = format!("{key} [{}]", arr.len());
            egui::CollapsingHeader::new(
                egui::RichText::new(header)
                    .monospace()
                    .color(palette.text_strong),
            )
            .id_salt((key, depth))
            .default_open(depth < 1)
            .show(ui, |ui| {
                for (idx, child) in arr.iter().take(500).enumerate() {
                    let item_key = format!("[{idx}]");
                    if !needle.is_empty() && !json_subtree_matches(&item_key, child, needle) {
                        continue;
                    }
                    show_json_node(ui, &item_key, child, depth + 1, needle, palette);
                }
                if arr.len() > 500 {
                    ui.small(format!("… +{} items", arr.len() - 500));
                }
            });
        }
        _ => {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("{key}:"))
                        .monospace()
                        .color(palette.text_dim),
                );
                let (txt, color) = match val {
                    Value::Null => ("null".to_owned(), palette.text_dim),
                    Value::Bool(b) => (b.to_string(), palette.warn),
                    Value::Number(n) => (n.to_string(), palette.accent),
                    Value::String(s) => (format!("{s:?}"), palette.text),
                    _ => unreachable!(),
                };
                ui.add(
                    egui::Label::new(egui::RichText::new(txt).monospace().color(color))
                        .selectable(true),
                );
            });
        }
    }
}

fn json_subtree_matches(key: &str, val: &Value, needle: &str) -> bool {
    if key.to_lowercase().contains(needle) {
        return true;
    }
    match val {
        Value::String(s) => s.to_lowercase().contains(needle),
        Value::Number(n) => n.to_string().contains(needle),
        Value::Bool(b) => b.to_string().contains(needle),
        Value::Array(arr) => arr
            .iter()
            .take(64)
            .any(|v| json_subtree_matches("", v, needle)),
        Value::Object(map) => map
            .iter()
            .take(64)
            .any(|(k, v)| json_subtree_matches(k, v, needle)),
        Value::Null => "null".contains(needle),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_and_tsv_parser_handles_rfc4180_quotes_and_delimiters() {
        let csv = "\u{feff}Name,Role,Note\r\nAlice,\"Lead, Rust\",\"Says \"\"Hi\"\"\"\r\nBob,Dev,\"Line 1\nLine 2\"";
        let sheet = parse_delimited_sheet(csv, None);
        assert_eq!(sheet.delimiter_label, "CSV (,)");
        assert_eq!(sheet.rows.len(), 3);
        assert_eq!(sheet.rows[1][1], "Lead, Rust");
        assert_eq!(sheet.rows[1][2], "Says \"Hi\"");
        assert_eq!(sheet.rows[2][2], "Line 1\nLine 2");

        assert_eq!(sheet_col_letter(0), "A");
        assert_eq!(sheet_col_letter(25), "Z");
        assert_eq!(sheet_col_letter(26), "AA");
    }

    #[test]
    fn jsonc_and_jsonl_parsers_strip_comments_and_count_invalid_lines() {
        let jsonc = r#"{
            // comment
            "url": "https://example.com/path", /* inline block */
            "items": [1, 2, ],
        }"#;
        let parsed = parse_json_or_jsonl(jsonc, "jsonc").unwrap();
        assert_eq!(parsed.root["url"], "https://example.com/path");
        assert_eq!(parsed.root["items"].as_array().unwrap().len(), 2);

        let jsonl = "{\"id\":1}\nnot-json\n{\"id\":2}\n";
        let jl = parse_json_or_jsonl(jsonl, "jsonl").unwrap();
        assert!(jl.is_jsonl);
        assert_eq!(jl.total_records, 2);
        assert_eq!(jl.invalid_lines, 1);
    }

    #[test]
    fn markdown_size_gate_and_mermaid_parser_cover_flowchart_and_sequence() {
        assert_eq!(
            classify_markdown_size(600 * 1024),
            MarkdownSizeDecision::AutoRender
        );
        assert_eq!(
            classify_markdown_size(700 * 1024),
            MarkdownSizeDecision::PromptUser {
                byte_len: 700 * 1024
            }
        );
        assert_eq!(
            classify_markdown_size(1_100 * 1024),
            MarkdownSizeDecision::SourceOnly {
                byte_len: 1_100 * 1024
            }
        );

        let md = "# Title\n\n```mermaid\nflowchart LR\n  A[\"Start\"] -->|Go| B{Check}\n  B --> C((Done))\n```\n\nFooter";
        let segs = extract_markdown_segments(md);
        assert_eq!(segs.len(), 3);
        let mermaid_segment = match &segs[1] {
            MarkdownSegment::Mermaid(s) => s,
            _ => panic!("expected mermaid"),
        };
        let diag = parse_mermaid_diagram(mermaid_segment).unwrap();
        match diag {
            MermaidDiagram::Flowchart {
                direction,
                nodes,
                edges,
            } => {
                assert_eq!(direction, FlowDirection::LeftRight);
                assert_eq!(nodes.len(), 3);
                assert_eq!(edges.len(), 2);
                assert_eq!(edges[0].label.as_deref(), Some("Go"));
            }
            _ => panic!("expected flowchart"),
        }

        let seq = "sequenceDiagram\n  participant UI as Tessel UI\n  UI->>API: Sign in\n  API-->>UI: Token";
        let sdiag = parse_mermaid_diagram(seq).unwrap();
        match sdiag {
            MermaidDiagram::Sequence {
                participants,
                messages,
            } => {
                assert_eq!(participants.len(), 2);
                assert_eq!(participants[0].label, "Tessel UI");
                assert_eq!(messages.len(), 2);
                assert!(messages[1].dashed);
            }
            _ => panic!("expected sequence"),
        }
    }
}
