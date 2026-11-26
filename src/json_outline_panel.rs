use crate::{files::Document, outline_tree::Symbol};
use eframe::egui;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Cache {
    document: u64,
    text: Arc<str>,
    pub(crate) symbols: Arc<Vec<Symbol>>,
}

impl Cache {
    pub(crate) fn matches(&self, doc: &Document) -> bool {
        self.document == doc.instance_id() && *self.text == doc.text
    }
    pub(crate) fn current(previous: Option<Self>, doc: &Document) -> Self {
        previous
            .filter(|cache| cache.matches(doc))
            .unwrap_or_else(|| {
                let mut symbols = crate::json_outline::document_symbols(&doc.text);
                crate::outline_tree::keyed(&mut symbols, "");
                Self {
                    document: doc.instance_id(),
                    text: Arc::from(doc.text.as_str()),
                    symbols: Arc::new(symbols),
                }
            })
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

pub fn show(ui: &mut egui::Ui, doc: &mut Document) {
    let id = ui.id().with("json-outline-cache");
    let cache = ui.data(|data| data.get_temp::<Cache>(id));
    let cache = Cache::current(cache, doc);
    ui.data_mut(|data| data.insert_temp(id, cache.clone()));
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
        if response.double_clicked() {
            reveal(doc, symbol.range, true);
        } else if response.clicked() {
            reveal(doc, symbol.selection_range.unwrap_or(symbol.range), false);
        }
        if open && !symbol.children.is_empty() {
            ui.indent((doc.instance_id(), &symbol.key), |ui| {
                draw(ui, &symbol.children, doc)
            });
        }
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
