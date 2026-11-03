//! Native `egui` keyboard shortcuts help panel (`HelpDialog.vue` / `helpShortcuts.js` parity).
//!
//! Composes directly with [`crate::shortcuts`] (`shortcut_groups`, `chords_of`, `fold_for_search`,
//! `route_help_dialog_escape`) while filtering strictly to the caller's `supported_keys` slice so
//! unimplemented shortcuts are never advertised, and adjusting native descriptions where current
//! Rust behavior is narrower than Electron (`Ctrl+Shift+T` opens the default shell; `Ctrl+Shift+P`
//! searches commands).
//!
//! Consumes `Escape` before both `egui::TextEdit` and `egui::ModalResponse::should_close`:
//! - when `query` is non-empty, `Escape` clears `query`, keeps focus in the search input, and
//!   returns `false` so the modal remains open;
//! - when `query` is empty, `Escape` returns `true` to close the modal.

use eframe::egui;

use crate::shortcuts::{
    FilteredShortcutGroup, FilteredShortcutRow, HelpEscapeOutcome, chords_of, fold_for_search,
    route_help_dialog_escape, shortcut_groups,
};

/// Returns the truthful native shortcut description for `keys`, overriding reference entries whose
/// native scope is currently narrower than Electron (`Ctrl+Shift+T` shell-only, `Ctrl+Shift+P`
/// commands-only) without mutating the reference [`crate::shortcuts`] specification.
#[must_use]
pub fn native_shortcut_text(
    keys: &str,
    reference_text: &'static str,
    french: bool,
) -> &'static str {
    match (keys, french) {
        ("Ctrl+Shift+T", false) => "New pane: default shell",
        ("Ctrl+Shift+T", true) => "Nouveau panneau : shell par défaut",
        ("Ctrl+Shift+P", false) => "Command palette: find commands",
        ("Ctrl+Shift+P", true) => "Palette de commandes : chercher une commande",
        _ => reference_text,
    }
}

/// Checks whether a shortcut row (`"Ctrl+Shift+E"` or multi-chord `"F7 / Shift+F7"`) is backed by
/// the caller's `supported_keys` slice.
#[must_use]
pub fn is_shortcut_supported(row_keys: &str, supported_keys: &[&str]) -> bool {
    if supported_keys
        .iter()
        .any(|k| k.trim().eq_ignore_ascii_case(row_keys))
    {
        return true;
    }
    row_keys.contains(" / ")
        && row_keys.split(" / ").all(|part| {
            supported_keys
                .iter()
                .any(|k| k.trim().eq_ignore_ascii_case(part))
        })
}

/// Filters `shortcut_groups(french)` down to `supported_keys`, applies truthful native text
/// overrides, and runs accent-insensitive EN/FR multi-word search over group title, description,
/// and localized keycap labels (`"maj+t"`, `"echap"`, `"fleches"`, etc.).
#[must_use]
pub fn filter_supported_shortcuts(
    french: bool,
    supported_keys: &[&str],
    query: &str,
) -> Vec<FilteredShortcutGroup> {
    let folded_query = fold_for_search(query);
    let words: Vec<&str> = folded_query
        .split(|c: char| c.is_whitespace() || c == '+')
        .filter(|w| !w.is_empty())
        .collect();

    shortcut_groups(french)
        .into_iter()
        .filter_map(|group| {
            let rows: Vec<FilteredShortcutRow> = group
                .rows
                .into_iter()
                .filter(|(keys, _)| is_shortcut_supported(keys, supported_keys))
                .map(|(keys, text)| FilteredShortcutRow {
                    keys,
                    text: native_shortcut_text(keys, text, french),
                    chords: chords_of(keys, french),
                })
                .filter(|row| {
                    if words.is_empty() {
                        return true;
                    }
                    let chords_text = row
                        .chords
                        .iter()
                        .map(|c| c.join(" "))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let hay =
                        fold_for_search(&format!("{} {} {}", group.title, row.text, chords_text));
                    words.iter().all(|w| hay.contains(w))
                })
                .collect();
            (!rows.is_empty()).then_some(FilteredShortcutGroup {
                title: group.title,
                rows,
            })
        })
        .collect()
}

fn balanced_column_split(groups: &[FilteredShortcutGroup]) -> usize {
    if groups.len() <= 1 {
        return groups.len();
    }
    let weights: Vec<usize> = groups.iter().map(|g| 2 + g.rows.len()).collect();
    let total: usize = weights.iter().sum();
    let mut best_idx = 1;
    let mut best_diff = usize::MAX;
    let mut left_sum = 0;
    for (idx, &w) in weights.iter().enumerate().take(groups.len() - 1) {
        left_sum += w;
        let right_sum = total.saturating_sub(left_sum);
        let diff = left_sum.abs_diff(right_sum);
        if diff < best_diff {
            best_diff = diff;
            best_idx = idx + 1;
        }
    }
    best_idx
}

fn render_keycap(ui: &mut egui::Ui, key: &str) {
    let visuals = ui.visuals();
    let bg = visuals.widgets.noninteractive.bg_fill;
    let stroke = visuals.widgets.noninteractive.bg_stroke;
    let text_color = visuals.strong_text_color();
    egui::Frame::new()
        .fill(bg)
        .stroke(stroke)
        .corner_radius(4.0_f32)
        .inner_margin(egui::Margin::symmetric(5, 2))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(key)
                        .monospace()
                        .size(11.0_f32)
                        .color(text_color),
                )
                .selectable(false)
                .extend(),
            );
        });
}

fn render_shortcut_row(ui: &mut egui::Ui, row: &FilteredShortcutRow, or_word: &str) {
    let weak_color = ui.visuals().weak_text_color();
    egui::Sides::new()
        .shrink_left()
        .wrap()
        .spacing(12.0_f32)
        .show(
            ui,
            |ui| {
                ui.label(row.text);
            },
            |ui| {
                ui.spacing_mut().item_spacing.x = 3.0_f32;
                for (ci, chord) in row.chords.iter().enumerate().rev() {
                    for (ki, key) in chord.iter().enumerate().rev() {
                        render_keycap(ui, key);
                        if ki > 0 {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new("+").size(10.5_f32).color(weak_color),
                                )
                                .selectable(false)
                                .extend(),
                            );
                        }
                    }
                    if ci > 0 {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(or_word)
                                    .size(10.5_f32)
                                    .color(weak_color),
                            )
                            .selectable(false)
                            .extend(),
                        );
                    }
                }
            },
        );
}

fn render_group_slice(ui: &mut egui::Ui, groups: &[FilteredShortcutGroup], or_word: &str) {
    let heading_color = ui.visuals().weak_text_color();
    for (gi, group) in groups.iter().enumerate() {
        if gi > 0 {
            ui.add_space(10.0_f32);
        }
        ui.label(
            egui::RichText::new(group.title.to_uppercase())
                .size(11.0_f32)
                .strong()
                .color(heading_color),
        );
        ui.add_space(4.0_f32);
        for row in &group.rows {
            render_shortcut_row(ui, row, or_word);
            ui.add_space(2.0_f32);
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct HelpPanel {
    query: String,
    focused: bool,
}

impl HelpPanel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears any search filter and schedules focus on the search input on the next frame.
    pub fn reset(&mut self) {
        self.query.clear();
        self.focused = false;
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// Renders the searchable shortcuts panel and returns `true` when `Escape` on an empty search
    /// requests closing the surrounding dialog.
    pub fn show(&mut self, ui: &mut egui::Ui, french: bool, supported_keys: &[&str]) -> bool {
        let (saw_escape, saw_fresh_escape) = ui.input_mut(|i| {
            let fresh = i.events.iter().any(|ev| {
                matches!(
                    ev,
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } if modifiers.matches_logically(egui::Modifiers::NONE)
                )
            });
            let consumed = i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            (consumed, fresh)
        });
        if saw_escape {
            self.focused = false;
        }

        let mut close_requested = false;
        if saw_fresh_escape {
            match route_help_dialog_escape(&self.query) {
                HelpEscapeOutcome::ClearQuery => {
                    self.query.clear();
                    self.focused = false;
                }
                HelpEscapeOutcome::CloseDialog => {
                    close_requested = true;
                }
            }
        }

        let search_placeholder = if french {
            "Chercher un raccourci"
        } else {
            "Search shortcuts"
        };
        let clear_label = if french {
            "Effacer la recherche"
        } else {
            "Clear search"
        };
        let no_match_label = if french {
            "Aucun raccourci ne correspond."
        } else {
            "No shortcut matches."
        };
        let or_word = if french { "ou" } else { "or" };

        let mut clear_clicked = false;
        let has_query = !self.query.is_empty();
        egui::Sides::new().shrink_left().spacing(6.0_f32).show(
            ui,
            |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .id(egui::Id::new("tessel-help-panel-search"))
                        .char_limit(128)
                        .desired_width(f32::INFINITY)
                        .hint_text(search_placeholder),
                );
                if !self.focused {
                    resp.request_focus();
                    if !ui.is_sizing_pass() {
                        self.focused = true;
                    }
                }
            },
            |ui| {
                if has_query && ui.small_button("×").on_hover_text(clear_label).clicked() {
                    clear_clicked = true;
                }
            },
        );
        if clear_clicked {
            self.query.clear();
            self.focused = false;
        }

        ui.add_space(10.0_f32);

        let shown = filter_supported_shortcuts(french, supported_keys, &self.query);
        if shown.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0_f32);
                ui.colored_label(ui.visuals().weak_text_color(), no_match_label);
                ui.add_space(8.0_f32);
            });
            return close_requested;
        }

        let scroll_max_h =
            (ui.ctx().content_rect().height() * 0.86_f32 - 125.0_f32).clamp(120.0_f32, 560.0_f32);
        egui::ScrollArea::vertical()
            .id_salt("help-panel-scroll")
            .auto_shrink([false, true])
            .max_height(scroll_max_h)
            .show(ui, |ui| {
                if ui.available_width() >= 600.0_f32 && shown.len() >= 2 {
                    let split = balanced_column_split(&shown);
                    let prev_gap = ui.spacing().item_spacing.x;
                    ui.spacing_mut().item_spacing.x = 24.0_f32;
                    ui.columns(2, |cols| {
                        render_group_slice(&mut cols[0], &shown[..split], or_word);
                        render_group_slice(&mut cols[1], &shown[split..], or_word);
                    });
                    ui.spacing_mut().item_spacing.x = prev_gap;
                } else {
                    render_group_slice(ui, &shown, or_word);
                }
            });

        close_requested
    }
}
