use crate::{
    files::Document,
    outline_tree::{Sort, Symbol},
};
use eframe::egui;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Cache {
    document: u64,
    sort: Sort,
    followed: Option<crate::files::OutlineCursor>,
    text: Arc<str>,
    pub(crate) symbols: Arc<Vec<Symbol>>,
}

impl Cache {
    pub(crate) fn matches(&self, doc: &Document) -> bool {
        self.document == doc.instance_id() && *self.text == doc.text
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
                let mut symbols = crate::json_outline::document_symbols(&doc.text);
                crate::outline_tree::keyed(&mut symbols, "");
                Self {
                    document: doc.instance_id(),
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
pub(crate) fn supported(doc: &Document) -> bool {
    !doc.media
        && doc.diff_title.is_none()
        && doc
            .path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("json") || e.eq_ignore_ascii_case("jsonc"))
}

pub(crate) const SORT_OPTIONS: [(Sort, &str, &str); 3] = [
    (Sort::Position, "Sort By: Position", "Trier par : position"),
    (Sort::Name, "Sort By: Name", "Trier par : nom"),
    (Sort::Kind, "Sort By: Category", "Trier par : catégorie"),
];

pub fn show(ui: &mut egui::Ui, doc: &mut Document, sort: Sort, follow: bool) {
    let id = ui.id().with("json-outline-cache");
    let cache = ui.data(|data| data.get_temp::<Cache>(id));
    let mut cache = Cache::current(cache, doc, sort);
    let scroll = cache.follow(doc, follow);
    ui.data_mut(|data| data.insert_temp(id, cache.clone()));
    let expand = crate::outline_tree::all_collapsed(&cache.symbols, &doc.tools.outline_folds);
    if ui
        .small_button(fold_all_label(expand, crate::i18n::is_french()))
        .clicked()
    {
        set_all_folds(doc, &cache.symbols, expand);
    }
    if cache.symbols.is_empty() {
        ui.label(if crate::i18n::is_french() {
            "Aucun symbole dans ce document."
        } else {
            "No symbols found in this document."
        });
        return;
    }
    egui::ScrollArea::vertical()
        .max_height(160.0)
        .show(ui, |ui| {
            draw(ui, &cache.symbols, doc, scroll);
        });
}

fn draw(ui: &mut egui::Ui, symbols: &[Symbol], doc: &mut Document, scroll: bool) {
    for symbol in symbols {
        let open = !doc.tools.outline_folds.contains(&symbol.key);
        let selected = doc.tools.outline_selected.as_ref() == Some(&symbol.key);
        let response = ui
            .horizontal(|ui| {
                if !symbol.children.is_empty()
                    && ui.small_button(if open { "▾" } else { "▸" }).clicked()
                {
                    toggle_fold(doc, &symbol.key);
                }
                ui.selectable_label(selected, &symbol.name)
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
            let range = if select {
                symbol.range
            } else {
                symbol.selection_range.unwrap_or(symbol.range)
            };
            reveal(doc, range, select);
        }
        if open && !symbol.children.is_empty() {
            ui.indent((doc.instance_id(), &symbol.key), |ui| {
                draw(ui, &symbol.children, doc, scroll)
            });
        }
    }
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
