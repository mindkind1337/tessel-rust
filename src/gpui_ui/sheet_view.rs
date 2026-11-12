//! Virtualized bounded spreadsheet preview, with a shared horizontal header/body scroll.
use super::{
    TesselGpuiRoot,
    theme::{GpuiThemeTokens, mono_font_family},
};
use crate::rich_previews::{MAX_SHEET_COLS, MAX_SHEET_ROWS, SheetTable, sheet_col_letter};
use gpui::{
    Context, FontWeight, InteractiveElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _, px, uniform_list,
};

const ROW_LABEL: f32 = 54.;
const CELL_WIDTH: f32 = 132.;
const ROW_HEIGHT: f32 = 24.;

fn dimensions(sheet: &SheetTable) -> (usize, usize, bool) {
    let rows = sheet.rows.len().min(MAX_SHEET_ROWS);
    let cols = sheet
        .rows
        .iter()
        .take(rows)
        .map(Vec::len)
        .max()
        .unwrap_or(0);
    (
        rows,
        cols.min(MAX_SHEET_COLS),
        sheet.truncated_rows > 0
            || sheet.truncated_cols > 0
            || rows < sheet.rows.len()
            || cols > MAX_SHEET_COLS,
    )
}

pub(crate) fn render(
    sheet: std::sync::Arc<SheetTable>,
    french: bool,
    tokens: &GpuiThemeTokens,
    cx: &Context<TesselGpuiRoot>,
) -> gpui::Div {
    let (row_count, cols, truncated) = dimensions(&sheet);
    let width = ROW_LABEL + cols as f32 * CELL_WIDTH;
    let mut header = div()
        .h(px(ROW_HEIGHT))
        .flex_shrink_0()
        .flex()
        .w(px(width))
        .bg(tokens.surface_2)
        .text_color(tokens.text_strong)
        .font_family(mono_font_family())
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(div().w(px(ROW_LABEL)).flex_shrink_0().px(px(6.)).child("#"));
    for col in 0..cols {
        header = header.child(
            div()
                .w(px(CELL_WIDTH))
                .flex_shrink_0()
                .px(px(6.))
                .overflow_hidden()
                .child(SharedString::from(sheet_col_letter(sheet.start_col + col))),
        );
    }
    // Only visible rows become GPUI elements. The bounded original cell strings remain intact.
    let data = sheet.clone();
    let palette = *tokens;
    let rows = uniform_list(
        "gpui-sheet-rows",
        row_count,
        cx.processor(move |_root, range: std::ops::Range<usize>, _, _cx| {
            range
                .map(|row| {
                    let mut line = div()
                        .h(px(ROW_HEIGHT))
                        .w(px(width))
                        .flex()
                        .items_center()
                        .bg(if row % 2 == 0 {
                            palette.term
                        } else {
                            palette.surface
                        })
                        .text_color(palette.text)
                        .text_size(px(11.5))
                        .child(
                            div()
                                .w(px(ROW_LABEL))
                                .flex_shrink_0()
                                .px(px(6.))
                                .font_family(mono_font_family())
                                .text_color(palette.text_dim)
                                .child(SharedString::from((data.start_row + row + 1).to_string())),
                        );
                    for col in 0..cols {
                        line = line.child(
                            div()
                                .w(px(CELL_WIDTH))
                                .flex_shrink_0()
                                .px(px(6.))
                                .border_l_1()
                                .border_color(palette.border)
                                .truncate()
                                .child(SharedString::from(
                                    data.rows[row].get(col).cloned().unwrap_or_default(),
                                )),
                        );
                    }
                    line
                })
                .collect()
        }),
    )
    .flex_1()
    .min_h_0()
    .w(px(width));
    div()
        .size_full()
        .min_h_0()
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .px(px(12.))
                .py(px(5.))
                .flex_shrink_0()
                .bg(tokens.chrome)
                .text_size(px(11.))
                .text_color(tokens.text_dim)
                .child(SharedString::from(format!(
                    "{} · {} {} × {} {}",
                    sheet.delimiter_label,
                    row_count,
                    if french { "lignes" } else { "rows" },
                    cols,
                    if french { "colonnes" } else { "columns" }
                )))
                .when(truncated, |el| {
                    el.child(if french {
                        " · Aperçu limité : le document contient davantage de données."
                    } else {
                        " · Preview limited: the document contains additional data."
                    })
                }),
        )
        .child(
            div()
                .id("gpui-sheet-horizontal")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_x_scroll()
                .child(
                    div()
                        .w(px(width))
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(header)
                        .child(rows),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn includes_rows_and_columns_beyond_the_old_silent_cutoff() {
        let sheet = SheetTable {
            rows: vec![vec!["value".into(); 200]; 5000],
            ..Default::default()
        };
        assert_eq!(dimensions(&sheet), (5000, 200, false));
        assert_eq!(sheet_col_letter(199), "GR");
    }
    #[test]
    fn oversized_and_decoder_truncated_tables_report_the_limit() {
        let mut sheet = SheetTable {
            rows: vec![vec![String::new(); 201]; 5001],
            ..Default::default()
        };
        assert_eq!(dimensions(&sheet), (5000, 200, true));
        sheet.rows = vec![vec![]];
        sheet.truncated_rows = 1;
        assert_eq!(dimensions(&sheet), (1, 0, true));
        assert_eq!(dimensions(&SheetTable::default()), (0, 0, false));
    }
}
