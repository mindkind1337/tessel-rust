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
    text: Arc<str>,
    pub(crate) symbols: Arc<Vec<Symbol>>,
}

impl Cache {
    pub(crate) fn matches(&self, doc: &Document) -> bool {
        self.document == doc.instance_id() && *self.text == doc.text
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

pub fn show(ui: &mut egui::Ui, doc: &mut Document, sort: Sort) {
    let id = ui.id().with("json-outline-cache");
    let cache = ui.data(|data| data.get_temp::<Cache>(id));
    let cache = Cache::current(cache, doc, sort);
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
            draw(ui, &cache.symbols, doc);
        });
}

fn draw(ui: &mut egui::Ui, symbols: &[Symbol], doc: &mut Document) {
    for symbol in symbols {
        let open = !doc.tools.outline_folds.contains(&symbol.key);
        let response = ui
            .horizontal(|ui| {
                if !symbol.children.is_empty()
                    && ui.small_button(if open { "▾" } else { "▸" }).clicked()
                {
                    toggle_fold(doc, &symbol.key);
                }
                ui.selectable_label(false, &symbol.name)
            })
            .inner;
        if response.clicked() {
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
                draw(ui, &symbol.children, doc)
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
