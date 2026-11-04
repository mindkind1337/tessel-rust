//! Native `egui` Settings navigation rail (`SettingsDialog.vue` `.set-side` parity).
//!
//! Pure UI component with zero filesystem, process, vault, or persistence side effects:
//! - Renders ONLY the sections explicitly injected by the caller in [`SettingsNavInput::entries`],
//!   grouped in canonical [`ALL_SETTINGS_GROUPS`] order (`AI`, `Workspace`, `Workflows`,
//!   `Insights`, `About`) via [`crate::settings_model`] — never advertising unimplemented pages.
//! - Top `.set-back` button (`"Back to app"` / `"Retour à l’application"`) with the exact
//!   `16x16` stroke arrow from `SettingsDialog.vue:365-373` (`M13 8H3.5M7.5 3.5L3 8l4.5 4.5`).
//! - Fixed-height `32px` `.set-search` input with vector [`Icon::Search`], focus-within accent
//!   border, bounded `256`-character query (`MAX_SETTINGS_SEARCH_CHARS`), and inline vector clear
//!   button when non-empty.
//! - Multi-word case-insensitive EN/FR search over injected titles, group headers, and caller
//!   keywords (preserving `Space` typing inside the search input).
//! - Keyboard contract (self-contained so callers do NOT need a separate shortcut helper):
//!   - Guarded by `ui.is_enabled() && !egui::Popup::is_any_open(ui.ctx())`.
//!   - `Ctrl+F` focuses the search input and selects its text.
//!   - `Escape` clears a non-empty search query on the first fresh press, and emits
//!     [`SettingsNavAction::Back`] only on a fresh press when the query is already empty
//!     (holding `Escape` across frames never clears and closes in one press).
//!   - `ArrowUp` / `ArrowDown` / `Enter` / `Space` are strictly focus-scoped to widgets inside
//!     [`SettingsNav`] and never steal keys from focused controls in the Settings page body.

use crate::icons::{self, Icon};
use crate::settings_model::{
    ALL_SETTINGS_GROUPS, MAX_SETTINGS_SEARCH_CHARS, SettingsGroupId, SettingsLang, SettingsPageId,
};
use eframe::egui::{
    self, Color32, FontId, Id, Key, Modifiers, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

/// Single implemented Settings section entry injected into [`SettingsNav`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsNavEntry {
    pub id: String,
    pub title: String,
    pub group: SettingsGroupId,
    pub keywords: String,
    pub icon: Icon,
}

/// Frame input for [`SettingsNav::show`].
#[derive(Debug, Clone, Copy)]
pub struct SettingsNavInput<'a> {
    pub entries: &'a [SettingsNavEntry],
    pub selected: &'a str,
    pub lang: SettingsLang,
}

/// User action emitted by [`SettingsNav::show`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsNavAction {
    Back,
    Select(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavFocusOwner {
    Back,
    Search,
    ClearSearch,
    List,
}

const fn back_label(lang: SettingsLang) -> &'static str {
    match lang {
        SettingsLang::En => "Back to app",
        SettingsLang::Fr => "Retour à l’application",
    }
}

const fn back_tooltip(lang: SettingsLang) -> &'static str {
    match lang {
        SettingsLang::En => "Back to app (Esc)",
        SettingsLang::Fr => "Retour à l’application (Échap)",
    }
}

const fn search_placeholder(lang: SettingsLang) -> &'static str {
    match lang {
        SettingsLang::En => "Search settings",
        SettingsLang::Fr => "Rechercher dans les paramètres",
    }
}

const fn search_tooltip(lang: SettingsLang) -> &'static str {
    match lang {
        SettingsLang::En => "Search settings (Ctrl+F)",
        SettingsLang::Fr => "Rechercher dans les paramètres (Ctrl+F)",
    }
}

const fn clear_search_label(lang: SettingsLang) -> &'static str {
    match lang {
        SettingsLang::En => "Clear search",
        SettingsLang::Fr => "Effacer la recherche",
    }
}

const fn nav_landmark_label(lang: SettingsLang) -> &'static str {
    match lang {
        SettingsLang::En => "Settings pages",
        SettingsLang::Fr => "Pages des paramètres",
    }
}

const fn empty_catalog_label(lang: SettingsLang) -> &'static str {
    match lang {
        SettingsLang::En => "No settings available.",
        SettingsLang::Fr => "Aucun paramètre disponible.",
    }
}

fn no_match_label(query: &str, lang: SettingsLang) -> String {
    let trimmed = query.trim();
    match lang {
        SettingsLang::En => format!("No settings match “{trimmed}”."),
        SettingsLang::Fr => format!("Aucun paramètre ne correspond à « {trimmed} »."),
    }
}

fn clamp_query_in_place(query: &mut String) {
    if query.chars().count() > MAX_SETTINGS_SEARCH_CHARS {
        *query = query.chars().take(MAX_SETTINGS_SEARCH_CHARS).collect();
    }
}

fn query_words(query: &str) -> Vec<String> {
    query
        .chars()
        .take(MAX_SETTINGS_SEARCH_CHARS)
        .collect::<String>()
        .trim()
        .to_lowercase()
        .split_whitespace()
        .map(ToString::to_string)
        .collect()
}

fn entry_matches_words(entry: &SettingsNavEntry, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let known_en = SettingsPageId::from_id(&entry.id).map_or("", SettingsPageId::title_en);
    let known_fr =
        SettingsPageId::from_id(&entry.id).map_or("", |page| page.title(SettingsLang::Fr));
    let haystack = format!(
        "{} {} {} {} {} {}",
        entry.title,
        known_en,
        known_fr,
        entry.group.title(SettingsLang::En),
        entry.group.title(SettingsLang::Fr),
        entry.keywords
    )
    .to_lowercase();
    words.iter().all(|w| haystack.contains(w))
}

/// Paints the exact `16x16` stroke back arrow from `SettingsDialog.vue:365-373`:
/// `d="M13 8H3.5M7.5 3.5L3 8l4.5 4.5"`, `stroke-width="1.5"`.
fn paint_back_arrow(painter: &Painter, rect: Rect, color: Color32) {
    let side = rect.width().min(rect.height());
    if !side.is_finite() || side <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let scale = side / 16.0_f32;
    let pt = |x: f32, y: f32| Pos2::new(sq.min.x + x * scale, sq.min.y + y * scale);
    let stroke = Stroke::new((1.5_f32 * scale).max(1.0_f32), color);
    painter.line_segment([pt(13.0_f32, 8.0_f32), pt(3.5_f32, 8.0_f32)], stroke);
    painter.add(egui::Shape::line(
        vec![
            pt(7.5_f32, 3.5_f32),
            pt(3.0_f32, 8.0_f32),
            pt(7.5_f32, 12.5_f32),
        ],
        stroke,
    ));
}

fn paint_single_line_ellipsis(
    painter: &Painter,
    rect: Rect,
    text: &str,
    font: FontId,
    color: Color32,
) {
    if rect.width() <= 1.0_f32 || rect.height() <= 1.0_f32 {
        return;
    }
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, rect.width());
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = painter.layout_job(job);
    let y = rect.center().y - galley.size().y * 0.5_f32;
    painter
        .with_clip_rect(rect)
        .galley(Pos2::new(rect.min.x, y), galley, color);
}

/// Native `egui` Settings navigation rail (`SettingsDialog.vue` `.set-side` parity).
#[derive(Debug, Default, Clone)]
pub struct SettingsNav {
    nav_id: Option<Id>,
    query: String,
    focused_entry_id: Option<String>,
    last_visible_ids: Vec<String>,
    focus_search_requested: bool,
    select_all_search_on_focus: bool,
    search_widget_id: Option<Id>,
    held_escape: bool,
    held_activate_key: Option<Key>,
}

impl SettingsNav {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the search query and resets transient focus/key-repeat state.
    pub fn reset(&mut self) {
        self.query.clear();
        self.focused_entry_id = None;
        self.last_visible_ids.clear();
        self.focus_search_requested = false;
        self.select_all_search_on_focus = false;
        self.held_escape = false;
        self.held_activate_key = None;
    }

    /// Returns the current bounded search query.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Replaces the search query, clamped to [`MAX_SETTINGS_SEARCH_CHARS`] (`256` chars).
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query
            .into()
            .chars()
            .take(MAX_SETTINGS_SEARCH_CHARS)
            .collect();
    }

    /// Requests keyboard focus on the search input on the next frame.
    pub fn focus_search(&mut self, ctx: &egui::Context) {
        self.focus_search_requested = true;
        self.select_all_search_on_focus = true;
        if let Some(id) = self.search_widget_id {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        ctx.request_repaint();
    }
    pub fn owns_focus(&self, ctx: &egui::Context) -> bool {
        let Some(id) = self.nav_id else {
            return false;
        };
        ctx.memory(|m| match m.focused() {
            Some(focused) => Self::classify_focus(Some(focused), id, true, true).is_some(),
            None => ["back", "search-input", "search-clear", "list"]
                .iter()
                .any(|suffix| m.had_focus_last_frame(id.with(*suffix))),
        })
    }

    fn ordered_visible_indices(entries: &[SettingsNavEntry], words: &[String]) -> Vec<usize> {
        let mut out = Vec::new();
        for group in ALL_SETTINGS_GROUPS {
            for (idx, entry) in entries.iter().enumerate() {
                if entry.group == group && entry_matches_words(entry, words) {
                    out.push(idx);
                }
            }
        }
        out
    }

    fn default_entry_id<'a>(
        entries: &'a [SettingsNavEntry],
        visible: &[usize],
        selected: &str,
    ) -> Option<&'a str> {
        if visible.is_empty() {
            return None;
        }
        if let Some(&idx) = visible.iter().find(|&&i| entries[i].id == selected) {
            return Some(entries[idx].id.as_str());
        }
        Some(entries[visible[0]].id.as_str())
    }

    fn classify_focus(
        focused: Option<Id>,
        nav_id: Id,
        has_clear: bool,
        has_list_items: bool,
    ) -> Option<NavFocusOwner> {
        let fid = focused?;
        if fid == nav_id.with("back") {
            Some(NavFocusOwner::Back)
        } else if fid == nav_id.with("search-input") {
            Some(NavFocusOwner::Search)
        } else if has_clear && fid == nav_id.with("search-clear") {
            Some(NavFocusOwner::ClearSearch)
        } else if has_list_items && fid == nav_id.with("list") {
            Some(NavFocusOwner::List)
        } else {
            None
        }
    }

    fn update_key_releases(&mut self, ui: &egui::Ui) {
        if !ui.is_enabled() || !ui.input(|i| i.focused) {
            self.held_escape = false;
            self.held_activate_key = None;
            return;
        }
        ui.input(|i| {
            if self.held_escape
                && i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        egui::Event::Key {
                            key: Key::Escape,
                            pressed: false,
                            ..
                        }
                    )
                })
            {
                self.held_escape = false;
            }
            if let Some(held) = self.held_activate_key
                && i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        egui::Event::Key {
                            key,
                            pressed: false,
                            ..
                        } if *key == held
                    )
                })
            {
                self.held_activate_key = None;
            }
        });
    }

    fn handle_modal_shortcuts(&mut self, ui: &mut egui::Ui) -> Option<SettingsNavAction> {
        if !ui.is_enabled() || egui::Popup::is_any_open(ui.ctx()) {
            return None;
        }

        let held_escape_before = self.held_escape;
        let (ctrl_f, fresh_escape) = ui.input_mut(|i| {
            let focus = i.consume_key(Modifiers::CTRL, Key::F);
            let fresh_esc = !held_escape_before
                && i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        egui::Event::Key {
                            key: Key::Escape,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if modifiers.matches_logically(Modifiers::NONE)
                    )
                });
            // Consume repeats too: holding Escape must never clear search and then close.
            let _ = i.consume_key(Modifiers::NONE, Key::Escape);
            (focus, fresh_esc)
        });

        if ctrl_f {
            self.focus_search_requested = true;
            self.select_all_search_on_focus = true;
        }

        if fresh_escape {
            self.held_escape = true;
            if self.query.is_empty() {
                return Some(SettingsNavAction::Back);
            }
            self.query.clear();
            self.focus_search_requested = true;
            self.focused_entry_id = None;
        }

        None
    }

    fn handle_focused_nav_keys(
        &mut self,
        ui: &mut egui::Ui,
        nav_id: Id,
        owner: NavFocusOwner,
        entries: &[SettingsNavEntry],
        visible: &[usize],
        selected: &str,
    ) -> (Option<SettingsNavAction>, bool, bool) {
        if !ui.is_enabled() || egui::Popup::is_any_open(ui.ctx()) {
            return (None, false, false);
        }

        let held_before = self.held_activate_key;
        let allow_space_activate = owner != NavFocusOwner::Search;
        let mut pressed_activate: Option<Key> = None;

        let (down, up) = ui.input_mut(|i| {
            for ev in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = ev
                    && modifiers.matches_logically(Modifiers::NONE)
                    && (*key == Key::Enter || (allow_space_activate && *key == Key::Space))
                    && held_before != Some(*key)
                {
                    pressed_activate = Some(*key);
                    break;
                }
            }
            let _ = i.consume_key(Modifiers::NONE, Key::Enter);
            if allow_space_activate {
                let _ = i.consume_key(Modifiers::NONE, Key::Space);
            }
            let down = i.consume_key(Modifiers::NONE, Key::ArrowDown);
            let up = i.consume_key(Modifiers::NONE, Key::ArrowUp);
            (down, up)
        });

        if let Some(k) = pressed_activate {
            self.held_activate_key = Some(k);
        }

        let mut action = None;
        let mut clear_query = false;
        let mut scroll_to_focused = false;

        if pressed_activate.is_some() {
            match owner {
                NavFocusOwner::Back => {
                    action = Some(SettingsNavAction::Back);
                }
                NavFocusOwner::ClearSearch => {
                    clear_query = true;
                }
                NavFocusOwner::Search | NavFocusOwner::List => {
                    let target_id = self
                        .focused_entry_id
                        .as_deref()
                        .filter(|id| visible.iter().any(|&i| entries[i].id == *id))
                        .or_else(|| Self::default_entry_id(entries, visible, selected))
                        .map(str::to_owned);
                    if let Some(id) = target_id {
                        self.focused_entry_id = Some(id.clone());
                        action = Some(SettingsNavAction::Select(id));
                    }
                }
            }
        }

        if (down || up) && !visible.is_empty() {
            let len = visible.len() as isize;
            let step: isize = if down { 1 } else { -1 };
            let cur_pos = self
                .focused_entry_id
                .as_deref()
                .and_then(|id| visible.iter().position(|&i| entries[i].id == id));

            let next_pos = match (owner, cur_pos) {
                (NavFocusOwner::List | NavFocusOwner::Search, Some(pos)) => {
                    (pos as isize + step).rem_euclid(len) as usize
                }
                _ => {
                    if owner == NavFocusOwner::Back || owner == NavFocusOwner::ClearSearch {
                        ui.ctx()
                            .memory_mut(|m| m.request_focus(nav_id.with("list")));
                    }
                    if let Some(def_id) = Self::default_entry_id(entries, visible, selected)
                        && let Some(pos) = visible.iter().position(|&i| entries[i].id == def_id)
                    {
                        if owner == NavFocusOwner::Search {
                            (pos as isize + step).rem_euclid(len) as usize
                        } else {
                            pos
                        }
                    } else if down {
                        0
                    } else {
                        visible.len() - 1
                    }
                }
            };
            self.focused_entry_id = Some(entries[visible[next_pos]].id.clone());
            scroll_to_focused = true;
        }

        (action, clear_query, scroll_to_focused)
    }

    /// Renders the Settings left navigation rail and returns any triggered [`SettingsNavAction`].
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        input: SettingsNavInput<'_>,
    ) -> Option<SettingsNavAction> {
        clamp_query_in_place(&mut self.query);
        self.update_key_releases(ui);

        let nav_id = ui.id().with("tessel-settings-nav");
        self.nav_id = Some(nav_id);
        let back_id = nav_id.with("back");
        let search_id = nav_id.with("search-input");
        let clear_id = nav_id.with("search-clear");
        let list_id = nav_id.with("list");
        self.search_widget_id = Some(search_id);

        let landmark_rect = Rect::from_min_size(ui.cursor().min, Vec2::ZERO);
        let landmark_resp = ui.interact(landmark_rect, nav_id.with("landmark"), Sense::hover());
        landmark_resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Other,
                ui.is_enabled(),
                nav_landmark_label(input.lang),
            )
        });

        let mut action = self.handle_modal_shortcuts(ui);

        let pre_words = query_words(&self.query);
        let pre_visible = Self::ordered_visible_indices(input.entries, &pre_words);
        let has_pre_items = !pre_visible.is_empty();
        if let Some(cur) = self.focused_entry_id.clone()
            && !pre_visible.iter().any(|&i| input.entries[i].id == cur)
        {
            self.focused_entry_id = if !pre_visible.is_empty()
                && let Some(old_pos) = self.last_visible_ids.iter().position(|id| *id == cur)
            {
                Some(
                    input.entries[pre_visible[old_pos.min(pre_visible.len() - 1)]]
                        .id
                        .clone(),
                )
            } else {
                Self::default_entry_id(input.entries, &pre_visible, input.selected)
                    .map(str::to_owned)
            };
        }

        if !has_pre_items && ui.is_enabled() && ui.ctx().memory(|m| m.focused()) == Some(list_id) {
            ui.ctx().memory_mut(|m| m.request_focus(search_id));
        }

        let has_clear_before = !self.query.is_empty();
        let focused_before = ui.ctx().memory(|m| m.focused());
        let focus_owner = if !ui.is_enabled() {
            self.focus_search_requested = false;
            self.select_all_search_on_focus = false;
            None
        } else {
            let classified =
                Self::classify_focus(focused_before, nav_id, has_clear_before, has_pre_items);
            if classified == Some(NavFocusOwner::List) && self.focused_entry_id.is_none() {
                self.focused_entry_id =
                    Self::default_entry_id(input.entries, &pre_visible, input.selected)
                        .map(str::to_owned);
            }
            if classified.is_none() {
                self.held_activate_key = None;
            }
            classified
        };

        let mut scroll_to_focused = false;
        if action.is_none()
            && let Some(owner) = focus_owner
        {
            let (key_act, clear_q, scroll_req) = self.handle_focused_nav_keys(
                ui,
                nav_id,
                owner,
                input.entries,
                &pre_visible,
                input.selected,
            );
            scroll_to_focused = scroll_req;
            if clear_q {
                self.query.clear();
                self.focused_entry_id = None;
                self.focus_search_requested = true;
            }
            if key_act.is_some() {
                action = key_act;
            }
        }

        ui.spacing_mut().item_spacing.y = 0.0_f32;

        // 1. `.set-back` ("Back to app" / "Retour à l’application").
        if Self::render_back_button(ui, back_id, input.lang) {
            action = Some(SettingsNavAction::Back);
        }

        ui.add_space(10.0_f32);

        // 2. `.set-search` fixed-height 32px search box (never shrinks in short windows).
        let previous_query = self.query.clone();
        self.render_search_box(ui, search_id, clear_id, input.lang);
        clamp_query_in_place(&mut self.query);
        if self.query != previous_query {
            let updated_words = query_words(&self.query);
            let updated_visible = Self::ordered_visible_indices(input.entries, &updated_words);
            self.focused_entry_id =
                Self::default_entry_id(input.entries, &updated_visible, input.selected)
                    .map(str::to_owned);
        }

        ui.add_space(6.0_f32);

        let words = query_words(&self.query);
        let is_searching = !words.is_empty();
        let visible = Self::ordered_visible_indices(input.entries, &words);

        // 3. Focusable list container (`Tab` stops once for the entire section list).
        let list_resp = if !visible.is_empty() {
            let list_viewport = ui.available_rect_before_wrap();
            Some(ui.interact(list_viewport, list_id, Sense::focusable_noninteractive()))
        } else {
            if ui.is_enabled() && ui.ctx().memory(|m| m.focused()) == Some(list_id) {
                ui.ctx().memory_mut(|m| m.request_focus(search_id));
            }
            None
        };

        let show_focus_ring_on = if ui.is_enabled() && !visible.is_empty() {
            let cur_focus = ui.ctx().memory(|m| m.focused());
            if cur_focus == Some(list_id) || (cur_focus == Some(search_id) && is_searching) {
                self.focused_entry_id.clone()
            } else {
                None
            }
        } else {
            None
        };

        // 4. Grouped section list inside bounded vertical scroll area.
        egui::ScrollArea::vertical()
            .id_salt(nav_id.with("scroll"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0_f32;

                if input.entries.is_empty() {
                    ui.add_space(14.0_f32);
                    let avail_w = ui.available_width().max(40.0_f32);
                    let (_, rect) = ui.allocate_space(Vec2::new(avail_w, 22.0_f32));
                    let text_rect = rect.shrink2(Vec2::new(10.0_f32, 0.0_f32));
                    paint_single_line_ellipsis(
                        ui.painter(),
                        text_rect,
                        empty_catalog_label(input.lang),
                        FontId::proportional(12.0_f32),
                        ui.visuals().weak_text_color(),
                    );
                    return;
                }

                if visible.is_empty() {
                    ui.add_space(14.0_f32);
                    let msg = no_match_label(&self.query, input.lang);
                    let avail_w = ui.available_width().max(40.0_f32);
                    let (_, rect) = ui.allocate_space(Vec2::new(avail_w, 24.0_f32));
                    let text_rect = rect.shrink2(Vec2::new(10.0_f32, 0.0_f32));
                    let status_resp = ui.interact(rect, nav_id.with("no-match"), Sense::hover());
                    status_resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &msg)
                    });
                    paint_single_line_ellipsis(
                        ui.painter(),
                        text_rect,
                        &msg,
                        FontId::proportional(12.0_f32),
                        ui.visuals().weak_text_color(),
                    );
                    status_resp.on_hover_text(msg);
                    return;
                }

                let mut first_group = true;
                for group in ALL_SETTINGS_GROUPS {
                    let group_has_any =
                        visible.iter().any(|&idx| input.entries[idx].group == group);
                    if !group_has_any {
                        continue;
                    }

                    ui.add_space(if first_group { 8.0_f32 } else { 14.0_f32 });
                    first_group = false;

                    // `.set-nav-head` uppercase group heading.
                    let head_text = group.title(input.lang).to_uppercase();
                    let full_w = ui.available_width().max(40.0_f32);
                    let (_, head_rect) = ui.allocate_space(Vec2::new(full_w, 18.0_f32));
                    let head_inner = Rect::from_min_max(
                        Pos2::new(head_rect.min.x + 10.0_f32, head_rect.min.y),
                        Pos2::new(
                            (head_rect.max.x - 6.0_f32).max(head_rect.min.x + 16.0_f32),
                            head_rect.max.y,
                        ),
                    );
                    paint_single_line_ellipsis(
                        ui.painter(),
                        head_inner,
                        &head_text,
                        FontId::proportional(10.5_f32),
                        ui.visuals().weak_text_color(),
                    );
                    ui.add_space(3.0_f32);

                    for &idx in &visible {
                        let entry = &input.entries[idx];
                        if entry.group != group {
                            continue;
                        }
                        let is_selected = entry.id == input.selected;
                        let is_key_focused =
                            show_focus_ring_on.as_deref() == Some(entry.id.as_str());
                        if self.render_nav_item(
                            ui,
                            nav_id,
                            list_id,
                            entry,
                            is_selected,
                            is_key_focused,
                            scroll_to_focused,
                        ) {
                            action = Some(SettingsNavAction::Select(entry.id.clone()));
                        }
                    }
                }
            });

        if let Some(list_resp) = list_resp {
            let active_id = self
                .focused_entry_id
                .as_deref()
                .filter(|id| visible.iter().any(|&i| input.entries[i].id == *id))
                .or_else(|| Self::default_entry_id(input.entries, &visible, input.selected));
            let active_title = active_id.and_then(|id| {
                input
                    .entries
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| e.title.as_str())
            });
            let list_a11y = match active_title {
                Some(t) => format!("{} — {t}", nav_landmark_label(input.lang)),
                None => nav_landmark_label(input.lang).to_owned(),
            };
            list_resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), &list_a11y)
            });
        }

        let end_focus = ui.ctx().memory(|m| m.focused());
        let has_clear_after = !self.query.is_empty();
        if ui.is_enabled()
            && !egui::Popup::is_any_open(ui.ctx())
            && Self::classify_focus(end_focus, nav_id, has_clear_after, !visible.is_empty())
                .is_some()
            && let Some(fid) = end_focus
        {
            ui.ctx().memory_mut(|m| {
                m.set_focus_lock_filter(
                    fid,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: false,
                    },
                );
            });
        }
        self.last_visible_ids = visible
            .iter()
            .map(|&idx| input.entries[idx].id.clone())
            .collect();

        action
    }

    fn render_back_button(ui: &mut egui::Ui, back_id: Id, lang: SettingsLang) -> bool {
        let label = back_label(lang);
        let tooltip = back_tooltip(lang);
        let full_w = ui.available_width().max(60.0_f32);
        let (_, row_rect) = ui.allocate_space(Vec2::new(full_w, 32.0_f32));
        let resp = ui.interact(row_rect, back_id, Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tooltip)
        });

        let has_focus = ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(back_id));
        let painter = ui.painter().with_clip_rect(row_rect);
        let accent = ui.visuals().selection.stroke.color;

        if resp.hovered() || has_focus {
            let bg = if resp.hovered() {
                ui.visuals().widgets.hovered.bg_fill
            } else {
                Color32::TRANSPARENT
            };
            let stroke = if has_focus {
                Stroke::new(1.5_f32, accent)
            } else {
                Stroke::NONE
            };
            painter.rect(row_rect, 6.0_f32, bg, stroke, StrokeKind::Inside);
        }

        let fg = if resp.hovered() || has_focus {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let icon_rect = Rect::from_center_size(
            Pos2::new(row_rect.min.x + 17.0_f32, row_rect.center().y),
            Vec2::splat(14.0_f32),
        );
        paint_back_arrow(&painter, icon_rect, fg);

        let text_rect = Rect::from_min_max(
            Pos2::new(icon_rect.max.x + 8.0_f32, row_rect.min.y),
            Pos2::new(
                (row_rect.max.x - 8.0_f32).max(icon_rect.max.x + 14.0_f32),
                row_rect.max.y,
            ),
        );
        paint_single_line_ellipsis(
            &painter,
            text_rect,
            label,
            FontId::proportional(13.0_f32),
            fg,
        );

        let clicked = resp.clicked();
        resp.on_hover_text(tooltip);
        if clicked {
            ui.ctx().memory_mut(|m| m.request_focus(back_id));
        }
        clicked
    }

    fn render_search_box(
        &mut self,
        ui: &mut egui::Ui,
        search_id: Id,
        clear_id: Id,
        lang: SettingsLang,
    ) {
        let placeholder = search_placeholder(lang);
        let tooltip = search_tooltip(lang);
        let full_w = ui.available_width().max(60.0_f32);
        let (_, box_rect) = ui.allocate_space(Vec2::new(full_w, 32.0_f32));

        let focus_requested_now = self.focus_search_requested;
        let select_all_now = self.select_all_search_on_focus;
        self.focus_search_requested = false;
        self.select_all_search_on_focus = false;

        let search_focused_before =
            ui.is_enabled() && (focus_requested_now || ui.ctx().memory(|m| m.has_focus(search_id)));

        let bg = ui.visuals().extreme_bg_color;
        let border_color = if search_focused_before {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke.color
        };
        ui.painter().with_clip_rect(box_rect).rect(
            box_rect,
            6.0_f32,
            bg,
            Stroke::new(1.0_f32, border_color),
            StrokeKind::Inside,
        );

        let icon_rect = Rect::from_center_size(
            Pos2::new(box_rect.min.x + 16.5_f32, box_rect.center().y),
            Vec2::splat(13.0_f32),
        );
        icons::paint(
            &ui.painter().with_clip_rect(box_rect),
            icon_rect,
            Icon::Search,
            ui.visuals().weak_text_color(),
        );

        let show_clear = !self.query.is_empty();
        let clear_rect = Rect::from_center_size(
            Pos2::new(box_rect.max.x - 15.0_f32, box_rect.center().y),
            Vec2::splat(18.0_f32),
        );
        let input_right = if show_clear {
            (clear_rect.min.x - 3.0_f32).max(icon_rect.max.x + 12.0_f32)
        } else {
            (box_rect.max.x - 8.0_f32).max(icon_rect.max.x + 12.0_f32)
        };
        let input_rect = Rect::from_min_max(
            Pos2::new(icon_rect.max.x + 7.0_f32, box_rect.min.y + 4.0_f32),
            Pos2::new(input_right, box_rect.max.y - 4.0_f32),
        );

        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(input_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.set_clip_rect(ui.clip_rect().intersect(input_rect));
        let edit_out = egui::TextEdit::singleline(&mut self.query)
            .id(search_id)
            .frame(false)
            .font(FontId::proportional(12.5_f32))
            .hint_text(placeholder)
            .char_limit(MAX_SETTINGS_SEARCH_CHARS)
            .desired_width(input_rect.width().max(8.0_f32))
            .show(&mut child);

        if focus_requested_now && ui.is_enabled() {
            edit_out.response.request_focus();
            if select_all_now {
                let mut state = edit_out.state;
                let char_len = self.query.chars().count();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(char_len),
                    )));
                state.store(ui.ctx(), search_id);
            }
        }
        edit_out.response.on_hover_text(tooltip);

        if show_clear {
            let clear_lbl = clear_search_label(lang);
            let clear_resp = ui.interact(clear_rect, clear_id, Sense::click());
            clear_resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), clear_lbl)
            });
            let clear_focused = ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(clear_id));
            let cp = ui.painter().with_clip_rect(clear_rect);
            if clear_resp.hovered() || clear_focused {
                let stroke = if clear_focused {
                    Stroke::new(1.0_f32, ui.visuals().selection.stroke.color)
                } else {
                    Stroke::NONE
                };
                cp.rect(
                    clear_rect,
                    4.0_f32,
                    ui.visuals().widgets.hovered.bg_fill,
                    stroke,
                    StrokeKind::Inside,
                );
            }
            let x_color = if clear_resp.hovered() || clear_focused {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().weak_text_color()
            };
            icons::paint(
                &cp,
                Rect::from_center_size(clear_rect.center(), Vec2::splat(11.0_f32)),
                Icon::Close,
                x_color,
            );
            let clear_clicked = clear_resp.clicked();
            clear_resp.on_hover_text(clear_lbl);
            if clear_clicked {
                self.query.clear();
                self.focused_entry_id = None;
                ui.ctx().memory_mut(|m| m.request_focus(search_id));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_nav_item(
        &mut self,
        ui: &mut egui::Ui,
        nav_id: Id,
        list_id: Id,
        entry: &SettingsNavEntry,
        is_selected: bool,
        is_key_focused: bool,
        scroll_to_focused: bool,
    ) -> bool {
        let full_w = ui.available_width().max(60.0_f32);
        let (_, item_rect) = ui.allocate_space(Vec2::new(full_w, 30.0_f32));
        let item_id = nav_id.with(("nav-item", entry.id.as_str()));
        let resp = ui.interact(item_rect, item_id, Sense::CLICK);
        resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                is_selected,
                &entry.title,
            )
        });

        if scroll_to_focused && is_key_focused {
            resp.scroll_to_me(Some(egui::Align::Center));
        }

        let painter = ui.painter().with_clip_rect(item_rect);
        let accent = ui.visuals().selection.stroke.color;

        if is_selected || resp.hovered() || is_key_focused {
            let bg = if is_selected {
                ui.visuals().widgets.active.bg_fill.gamma_multiply(0.75_f32)
            } else if resp.hovered() {
                ui.visuals()
                    .widgets
                    .hovered
                    .bg_fill
                    .gamma_multiply(0.55_f32)
            } else {
                Color32::TRANSPARENT
            };
            let stroke = if is_key_focused {
                Stroke::new(1.5_f32, accent)
            } else {
                Stroke::NONE
            };
            painter.rect(item_rect, 6.0_f32, bg, stroke, StrokeKind::Inside);
        }

        // `.set-nav-item.on svg { color: var(--accent); }` (`style.css:3516-3518`).
        let icon_color = if is_selected {
            accent
        } else if resp.hovered() || is_key_focused {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let icon_rect = Rect::from_center_size(
            Pos2::new(item_rect.min.x + 17.5_f32, item_rect.center().y),
            Vec2::splat(15.0_f32),
        );
        icons::paint(&painter, icon_rect, entry.icon, icon_color);

        let text_color = if is_selected || is_key_focused {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        let title_rect = Rect::from_min_max(
            Pos2::new(icon_rect.max.x + 10.0_f32, item_rect.min.y),
            Pos2::new(
                (item_rect.max.x - 8.0_f32).max(icon_rect.max.x + 16.0_f32),
                item_rect.max.y,
            ),
        );
        paint_single_line_ellipsis(
            &painter,
            title_rect,
            &entry.title,
            FontId::proportional(13.0_f32),
            text_color,
        );

        let clicked = resp.clicked();
        resp.on_hover_text(&entry.title);
        if clicked {
            self.focused_entry_id = Some(entry.id.clone());
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
        }
        clicked
    }
}
