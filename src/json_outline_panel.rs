use crate::{
    files::Document,
    outline_tree::{Sort, Symbol},
};
use eframe::egui;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Cache {
    document: u64,
    provider: Option<Provider>,
    sort: Sort,
    followed: Option<crate::files::OutlineCursor>,
    text: Arc<str>,
    pub(crate) symbols: Arc<Vec<Symbol>>,
}

impl Cache {
    pub(crate) fn matches(&self, doc: &Document) -> bool {
        self.document == doc.instance_id()
            && self.provider == provider(doc)
            && *self.text == doc.text
    }
    pub(crate) fn follow(&mut self, doc: &mut Document, enabled: bool) -> bool {
        let cursor = doc.tools.outline_cursor.filter(|_| enabled);
        if self.followed == cursor {
            return false;
        }
        self.followed = cursor;
        let Some(cursor) = cursor else { return false };
        let byte = match cursor {
            crate::files::OutlineCursor::Bytes(i) => {
                doc.text.floor_char_boundary(i.min(doc.text.len()))
            }
            crate::files::OutlineCursor::Chars(i) => doc
                .text
                .char_indices()
                .nth(i)
                .map_or(doc.text.len(), |(offset, _)| offset),
        };
        let mut line = 1;
        let mut column = 1;
        let mut chars = doc.text[..byte].chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' || c == '\n' {
                if c == '\r' {
                    chars.next_if_eq(&'\n');
                }
                line += 1;
                column = 1;
            } else {
                column += c.len_utf16();
            }
        }
        let mut path = crate::outline_tree::symbol_path_at(&self.symbols, line, column);
        let Some(key) = path.pop() else { return false };
        for ancestor in path {
            doc.tools.outline_folds.remove(&ancestor);
        }
        if doc.tools.outline_selected.as_ref() == Some(&key) {
            return false;
        }
        doc.tools.outline_selected = Some(key);
        true
    }

    pub(crate) fn current(previous: Option<Self>, doc: &Document, sort: Sort) -> Self {
        let mut cache = previous
            .filter(|cache| cache.matches(doc))
            .unwrap_or_else(|| {
                let mut symbols = match provider(doc) {
                    Some(Provider::Html) => crate::html_outline::document_symbols(&doc.text),
                    Some(Provider::Css) => crate::css_outline::document_symbols(&doc.text),
                    Some(Provider::Json) => crate::json_outline::document_symbols(&doc.text),
                    None => Vec::new(),
                };
                crate::outline_tree::keyed(&mut symbols, "");
                Self {
                    document: doc.instance_id(),
                    provider: provider(doc),
                    sort: Sort::Position,
                    followed: None,
                    text: Arc::from(doc.text.as_str()),
                    symbols: Arc::new(symbols),
                }
            });
        if cache.sort != sort {
            crate::outline_tree::sort_symbols(
                Arc::make_mut(&mut cache.symbols).as_mut_slice(),
                sort,
            );
            cache.sort = sort;
        }
        cache
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Provider {
    Json,
    Html,
    Css,
}
fn provider(doc: &Document) -> Option<Provider> {
    let ext = doc.path.extension()?.to_str()?;
    if ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm") {
        Some(Provider::Html)
    } else if ext.eq_ignore_ascii_case("json") || ext.eq_ignore_ascii_case("jsonc") {
        Some(Provider::Json)
    } else if ext.eq_ignore_ascii_case("css") {
        Some(Provider::Css)
    } else {
        None
    }
}
pub(crate) fn supported(doc: &Document) -> bool {
    !doc.media && doc.diff_title.is_none() && provider(doc).is_some()
}

pub(crate) const SORT_OPTIONS: [(Sort, &str, &str); 3] = [
    (Sort::Position, "Sort By: Position", "Trier par : position"),
    (Sort::Name, "Sort By: Name", "Trier par : nom"),
    (Sort::Kind, "Sort By: Category", "Trier par : catégorie"),
];

#[derive(Clone, Default)]
struct Find {
    document: u64,
    text: Option<String>,
}

pub fn show(ui: &mut egui::Ui, doc: &mut Document, sort: Sort, follow: bool, filter: bool) {
    let id = ui.id().with("json-outline-cache");
    let cache = ui.data(|data| data.get_temp::<Cache>(id));
    let mut cache = Cache::current(cache, doc, sort);

    let expand = crate::outline_tree::all_collapsed(&cache.symbols, &doc.tools.outline_folds);
    if ui
        .small_button(fold_all_label(expand, crate::i18n::is_french()))
        .clicked()
    {
        set_all_folds(doc, &cache.symbols, expand);
    }
    if cache.symbols.is_empty() {
        ui.data_mut(|d| d.insert_temp(id, cache));
        ui.label(if crate::i18n::is_french() {
            "Aucun symbole dans ce document."
        } else {
            "No symbols found in this document."
        });
        return;
    }
    let tree_id = ui.id().with(("outline-tree", doc.instance_id()));
    ui.interact(
        egui::Rect::NOTHING,
        tree_id,
        egui::Sense::focusable_noninteractive(),
    );
    let find_id = ui.id().with("outline-find");
    let mut find = ui
        .data_mut(|d| d.get_temp::<Find>(find_id))
        .unwrap_or_default();
    if find.document != doc.instance_id() {
        find = Find {
            document: doc.instance_id(),
            text: None,
        };
    }
    let input_id = tree_id.with("input");
    if ui.memory(|m| m.has_focus(tree_id)) {
        let seed = ui.input_mut(|i| {
            let mut seed = String::new();
            if !i.modifiers.command && !i.modifiers.ctrl && !i.modifiers.alt {
                i.events.retain(|event| {
                    if let egui::Event::Text(s) = event
                        && (!seed.is_empty() || !s.trim().is_empty())
                    {
                        seed.push_str(s);
                        return false;
                    }
                    true
                });
            }
            seed
        });
        if !seed.is_empty() {
            find.text.get_or_insert_with(String::new).push_str(&seed);
            ui.memory_mut(|m| m.request_focus(input_id));
        }
    }
    let french = crate::i18n::is_french();
    let mut select_first = false;
    let escape = find.text.is_some()
        && ui.memory(|m| {
            [tree_id, input_id]
                .iter()
                .any(|id| m.has_focus(*id) || m.had_focus_last_frame(*id))
        })
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if escape {
        find.text = None;
        ui.memory_mut(|m| m.request_focus(tree_id));
    }
    if let Some(text) = &mut find.text {
        let keyboard = ui.memory(|m| m.has_focus(input_id));
        let close_key = keyboard
            && ui.input_mut(|i| {
                text.is_empty() && i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
            });
        select_first = keyboard
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
        let close = ui
            .horizontal(|ui| {
                let hint = match (french, filter) {
                    (true, true) => "Taper pour filtrer",
                    (true, false) => "Taper pour rechercher",
                    (false, true) => "Type to filter",
                    (false, false) => "Type to search",
                };
                ui.add(
                    egui::TextEdit::singleline(text)
                        .id(input_id)
                        .desired_width((ui.available_width() - 28.).max(32.))
                        .hint_text(hint),
                );
                ui.small_button("×").clicked()
            })
            .inner;
        if close || close_key {
            find.text = None;
        }
        if close || close_key || select_first {
            ui.memory_mut(|m| m.request_focus(tree_id));
        }
    }
    ui.memory_mut(|m| {
        for id in [tree_id, input_id] {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                    tab: false,
                },
            );
        }
    });
    let query = find.text.as_deref().unwrap_or_default().trim();
    let scroll = query.is_empty() && cache.follow(doc, follow);
    let rows =
        crate::outline_tree::visible_rows(&cache.symbols, &doc.tools.outline_folds, query, !filter);
    let navigate =
        ui.memory(|m| m.has_focus(tree_id)) && tree_keys(ui, doc, &rows, !query.is_empty());
    if select_first && let Some(first) = rows.first() {
        doc.tools.outline_selected = Some(first.row.key.clone());
    }
    if !query.is_empty() && !rows.iter().any(|r| !r.row.matches.is_empty()) {
        ui.label(if french {
            "Aucun résultat trouvé."
        } else {
            "No results found."
        });
    }
    ui.data_mut(|d| {
        d.insert_temp(find_id, find);
        d.insert_temp(id, cache.clone());
    });
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .show(ui, |ui| {
            draw(ui, &rows, doc, scroll || navigate || select_first, tree_id);
        });
}

fn tree_keys(
    ui: &egui::Ui,
    doc: &mut Document,
    rows: &[crate::outline_tree::VisibleRow<'_>],
    searching: bool,
) -> bool {
    use egui::Key;
    let key = ui.input_mut(|i| {
        [
            Key::ArrowDown,
            Key::ArrowUp,
            Key::Home,
            Key::End,
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::Enter,
        ]
        .into_iter()
        .find(|key| i.consume_key(egui::Modifiers::NONE, *key))
    });
    key.is_some_and(|key| navigate(doc, rows, searching, key))
}

pub(crate) fn navigate(
    doc: &mut Document,
    rows: &[crate::outline_tree::VisibleRow<'_>],
    searching: bool,
    key: egui::Key,
) -> bool {
    use egui::Key;
    let Some(last) = rows.len().checked_sub(1) else {
        return false;
    };
    let current = rows
        .iter()
        .position(|entry| Some(&entry.row.key) == doc.tools.outline_selected.as_ref());
    let index = match key {
        Key::ArrowDown => current.map_or(0, |i| (i + 1).min(last)),
        Key::ArrowUp => current.map_or(last, |i| i.saturating_sub(1)),
        Key::Home => 0,
        Key::End => last,
        _ => {
            let Some(i) = current else { return false };
            let entry = &rows[i];
            match key {
                Key::Enter => {
                    reveal(
                        doc,
                        entry.symbol.selection_range.unwrap_or(entry.symbol.range),
                        false,
                    );
                    return false;
                }
                Key::ArrowRight if entry.row.has_children && !entry.row.open => {
                    doc.tools.outline_folds.remove(&entry.row.key);
                    return false;
                }
                Key::ArrowRight if entry.row.has_children => (i + 1).min(last),
                Key::ArrowLeft if entry.row.open && !searching => {
                    doc.tools.outline_folds.insert(entry.row.key.clone());
                    return false;
                }
                Key::ArrowLeft if entry.row.parent.is_some() => {
                    doc.tools.outline_selected.clone_from(&entry.row.parent);
                    return true;
                }
                _ => return false,
            }
        }
    };
    doc.tools.outline_selected = Some(rows[index].row.key.clone());
    true
}

fn draw(
    ui: &mut egui::Ui,
    rows: &[crate::outline_tree::VisibleRow<'_>],
    doc: &mut Document,
    scroll: bool,
    tree_id: egui::Id,
) {
    for entry in rows {
        let symbol = entry.symbol;
        let open = entry.row.open;
        let selected = doc.tools.outline_selected.as_ref() == Some(&symbol.key);
        let response = ui
            .push_id(&symbol.key, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(entry.row.depth as f32 * ui.spacing().indent);
                    if !symbol.children.is_empty()
                        && ui.small_button(if open { "▾" } else { "▸" }).clicked()
                    {
                        toggle_fold(doc, &symbol.key);
                    }
                    let label = highlighted_name(ui, &symbol.name, &entry.row.matches);
                    ui.selectable_label(selected, label)
                })
                .inner
            })
            .inner;
        if selected && scroll && !ui.clip_rect().contains_rect(response.rect) {
            response.scroll_to_me(Some(egui::Align::Center));
        }
        if response.clicked() {
            doc.tools.outline_selected = Some(symbol.key.clone());
            // egui counts successive clicks across widgets, including the section header.
            // Keep the click pair tied to the document and symbol the user activated.
            let history = ui.id().with(("outline-last-click", doc.instance_id()));
            let now = ui.input(|i| i.time);
            let delay = ui.ctx().options(|o| o.input_options.max_double_click_delay);
            let select = response.clicked_by(egui::PointerButton::Primary)
                && ui
                    .data(|d| d.get_temp::<(egui::Id, f64)>(history))
                    .is_some_and(|(row, time)| row == response.id && now - time < delay)
                && (response.double_clicked() || response.triple_clicked());
            ui.data_mut(|d| {
                if select {
                    d.remove::<(egui::Id, f64)>(history);
                } else {
                    d.insert_temp(history, (response.id, now));
                }
            });
            if !select {
                ui.memory_mut(|m| m.request_focus(tree_id));
            }
            let range = if select {
                symbol.range
            } else {
                symbol.selection_range.unwrap_or(symbol.range)
            };
            reveal(doc, range, select);
        }
    }
}

fn highlighted_name(ui: &egui::Ui, name: &str, matches: &[[usize; 2]]) -> egui::WidgetText {
    if matches.is_empty() {
        return name.to_owned().into();
    }
    let mut job = egui::text::LayoutJob::default();
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = egui::TextFormat::simple(font.clone(), ui.visuals().text_color());
    let hit = egui::TextFormat::simple(font, ui.visuals().hyperlink_color);
    let mut offset = 0;
    for span in crate::outline_tree::highlight_ranges(name, matches) {
        if span.start > offset {
            job.append(&name[offset..span.start], 0., plain.clone());
        }
        job.append(&name[span.clone()], 0., hit.clone());
        offset = span.end;
    }
    if offset < name.len() {
        job.append(&name[offset..], 0., plain);
    }
    job.into()
}

pub(crate) fn fold_all_label(expand: bool, french: bool) -> &'static str {
    match (expand, french) {
        (true, true) => "Tout développer",
        (false, true) => "Tout réduire",
        (true, false) => "Expand All",
        (false, false) => "Collapse All",
    }
}

pub(crate) fn set_all_folds(doc: &mut Document, symbols: &[Symbol], expand: bool) {
    doc.tools.outline_folds.clear();
    if !expand {
        doc.tools
            .outline_folds
            .extend(crate::outline_tree::parent_keys(symbols));
    }
}

pub fn toggle_fold(doc: &mut Document, key: &str) {
    if !doc.tools.outline_folds.remove(key) {
        doc.tools.outline_folds.insert(key.to_owned());
    }
}

pub fn reveal(doc: &mut Document, range: crate::outline_tree::Range, select: bool) {
    if provider(doc) == Some(Provider::Html) {
        doc.preview_mode = false;
        doc.tools.html_plain_text = None;
    }
    let start =
        crate::json_outline::char_offset(&doc.text, range.start_line_number, range.start_column);
    let end = if select {
        crate::json_outline::char_offset(&doc.text, range.end_line_number, range.end_column)
    } else {
        start
    };
    doc.tools.outline_reveal_focus = Some(select);
    doc.tools.pending_char_range = Some((start, end));
    doc.tools.pending_scroll_line = Some(range.start_line_number.saturating_sub(1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::OutlineCursor;

    #[test]
    fn html_outline_reveals_the_selected_element_in_the_unsaved_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.HTM");
        std::fs::write(&path, "<main/>").unwrap();
        let mut doc = Document::open(dir.path(), &path).unwrap();
        doc.text = "<main>😀<p id=one>body</p></main>".into();
        assert!(supported(&doc));
        let cache = Cache::current(None, &doc, Sort::Position);
        let target = &cache.symbols[0].children[0];
        assert_eq!(target.name, "p#one");
        reveal(&mut doc, target.range, true);
        assert!(!doc.preview_mode);
        let (a, b) = doc.tools.pending_char_range.unwrap();
        assert_eq!(
            doc.text.chars().skip(a).take(b - a).collect::<String>(),
            "<p id=one>body</p>"
        );
    }
    #[test]
    fn outline_follow_maps_both_editor_offsets_and_respects_manual_folds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("offsets.json");
        let text = "{\r\n\"😀\":{\"é\":null},\"end\":3}";
        std::fs::write(&path, text).unwrap();
        let byte = text.find("null").unwrap();
        for cursor in [
            OutlineCursor::Bytes(byte),
            OutlineCursor::Chars(text[..byte].chars().count()),
        ] {
            let mut doc = Document::open(dir.path(), &path).unwrap();
            let mut cache = Cache::current(None, &doc, Sort::Name);
            set_all_folds(&mut doc, &cache.symbols, false);
            doc.tools.outline_cursor = Some(cursor);
            assert!(!cache.follow(&mut doc, false));
            assert!(cache.follow(&mut doc, true));
            assert_eq!(doc.tools.outline_selected.as_deref(), Some("/1:😀/12:é"));
            assert!(doc.tools.outline_folds.is_empty());
            set_all_folds(&mut doc, &cache.symbols, false);
            assert!(!cache.follow(&mut doc, true));
            assert!(doc.tools.outline_folds.contains("/1:😀"));
        }
    }
}
