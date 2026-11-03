//! Native `egui` 4-column Kanban Task Board (`TaskBoard.vue` + `TaskCard.vue` parity).
//!
//! Pure UI component with zero filesystem, process, network, or persistence side effects:
//! - Renders ONLY the real native task fields (`id`, `title`, `column` in `0..=3`) without
//!   invented assignee, priority, cost, or orchestration controls.
//! - Matches `TaskBoard.vue`, `TaskCard.vue`, `style.css:1373-1708`, and `tasks.json`:
//!   - Top `.task-board-add` bar (`32px` input bounded to [`MAX_TASK_TITLE_CHARS`] = `1000` chars
//!     + `"Add"` / `"Ajouter"` submit button refusing empty/whitespace-only titles).
//!   - 4 canonical columns (`0 = To do`, `1 = Doing`, `2 = Review`, `3 = Done`) with exact
//!     reference colors (`#8b93a1`, `#6c9cff`, `#f5a524`, `#3ddc84`), `8px` status dots,
//!     rounded count pills, dashed empty placeholders (`"No tasks"` / `"Aucune tâche"`), and
//!     cards with `3px` column-colored left accent bars.
//!   - Adapts cleanly to narrow right `SidePanel` widths (`280px`, stacked scrollable sections
//!     exactly like `style.css:1437-1446`) and wide viewports (`>= 720px`, 4 equal columns)
//!     without ever imposing `880px` on the parent `Ui`.
//!   - Vector chevron move buttons (bounded to `0..=3`), pointer drag-and-drop between columns,
//!     and strictly focus-scoped keyboard navigation (`ArrowUp`/`ArrowDown` to select cards,
//!     `Alt+ArrowLeft`/`Alt+ArrowRight` or `ArrowLeft`/`ArrowRight` on the focused card list to
//!     move one column with held-key anti-repeat, respecting `!ui.is_enabled()` and open popups).

use eframe::egui::{
    self, Color32, FontId, Id, Key, Modifiers, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

/// Maximum task title length in Unicode scalar values (matches `app.rs` `1000`-char cap).
pub const MAX_TASK_TITLE_CHARS: usize = 1000;

/// Number of Kanban columns (`0 = To do`, `1 = Doing`, `2 = Review`, `3 = Done`).
pub const TASK_COLUMN_COUNT: usize = 4;

/// Single task card rendered in [`TasksBoard`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCard {
    pub id: u64,
    pub title: String,
    pub column: usize,
}

/// Frame input for [`TasksBoard::show`].
#[derive(Debug, Clone, Copy)]
pub struct TasksBoardInput<'a> {
    pub tasks: &'a [TaskCard],
    pub french: bool,
    pub project_selected: bool,
}

/// User action emitted by [`TasksBoard::show`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TasksBoardAction {
    Add(String),
    Move { id: u64, column: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoardFocusOwner {
    Input,
    AddButton,
    List,
}

const fn clamp_column(column: usize) -> usize {
    if column >= TASK_COLUMN_COUNT {
        TASK_COLUMN_COUNT - 1
    } else {
        column
    }
}

/// Exact `--col` palette from `C:\Tessel\src\renderer\src\style.css:1474-1485`.
const fn column_color(column: usize) -> Color32 {
    match clamp_column(column) {
        0 => Color32::from_rgb(0x8b, 0x93, 0xa1), // todo:   #8b93a1
        1 => Color32::from_rgb(0x6c, 0x9c, 0xff), // doing:  #6c9cff
        2 => Color32::from_rgb(0xf5, 0xa5, 0x24), // review: #f5a524
        _ => Color32::from_rgb(0x3d, 0xdc, 0x84), // done:   #3ddc84
    }
}

/// Exact column titles from `TaskBoard.vue:174-180` and `i18n/locales/fr/tasks.json:3-8`.
const fn column_label(column: usize, french: bool) -> &'static str {
    match (clamp_column(column), french) {
        (0, false) => "To do",
        (0, true) => "À faire",
        (1, false) => "Doing",
        (1, true) => "En cours",
        (2, false) => "Review",
        (2, true) => "À examiner",
        (_, false) => "Done",
        (_, true) => "Terminé",
    }
}

const fn add_placeholder(french: bool) -> &'static str {
    if french {
        "Ajouter une tâche…"
    } else {
        "Add a task…"
    }
}

const fn add_button_label(french: bool) -> &'static str {
    if french { "Ajouter" } else { "Add" }
}

const fn empty_column_label(french: bool) -> &'static str {
    if french { "Aucune tâche" } else { "No tasks" }
}

const fn select_project_label(french: bool) -> &'static str {
    if french {
        "Sélectionnez d’abord un projet."
    } else {
        "Select a project first."
    }
}

const fn board_landmark_label(french: bool) -> &'static str {
    if french {
        "Tableau des tâches"
    } else {
        "Task board"
    }
}

fn move_button_label(target_col: usize, french: bool) -> String {
    let col_name = column_label(target_col, french);
    if french {
        format!("Déplacer vers {col_name}")
    } else {
        format!("Move to {col_name}")
    }
}

fn clamp_draft_in_place(draft: &mut String) {
    if draft.chars().count() > MAX_TASK_TITLE_CHARS {
        *draft = draft.chars().take(MAX_TASK_TITLE_CHARS).collect();
    }
}

fn sanitized_submit_title(draft: &str) -> Option<String> {
    let trimmed: String = draft.trim().chars().take(MAX_TASK_TITLE_CHARS).collect();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn key_held_after_events(events: &[egui::Event], target: Key) -> bool {
    let mut held = false;
    for ev in events {
        if let egui::Event::Key { key, pressed, .. } = ev
            && *key == target
        {
            held = *pressed;
        }
    }
    held
}

fn paint_chevron_horizontal(painter: &Painter, rect: Rect, color: Color32, left: bool) {
    let side = rect.width().min(rect.height());
    if !side.is_finite() || side <= 0.0_f32 {
        return;
    }
    let sq = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let scale = side / 16.0_f32;
    let pt = |x: f32, y: f32| Pos2::new(sq.min.x + x * scale, sq.min.y + y * scale);
    let stroke = Stroke::new((1.6_f32 * scale).max(1.0_f32), color);
    let points = if left {
        vec![
            pt(10.0_f32, 3.5_f32),
            pt(5.5_f32, 8.0_f32),
            pt(10.0_f32, 12.5_f32),
        ]
    } else {
        vec![
            pt(6.0_f32, 3.5_f32),
            pt(10.5_f32, 8.0_f32),
            pt(6.0_f32, 12.5_f32),
        ]
    };
    painter.add(egui::Shape::line(points, stroke));
}

fn paint_dashed_rect(painter: &Painter, rect: Rect, stroke: Stroke, dash: f32, gap: f32) {
    if rect.width() <= 2.0_f32 || rect.height() <= 2.0_f32 || stroke.width <= 0.0_f32 {
        return;
    }
    let step = (dash + gap).max(2.0_f32);
    let draw_edge = |start: Pos2, end: Pos2| {
        let delta = end - start;
        let len = delta.length();
        if len <= 0.5_f32 {
            return;
        }
        let dir = delta / len;
        let mut t = 0.0_f32;
        while t < len {
            let t_end = (t + dash).min(len);
            painter.line_segment([start + dir * t, start + dir * t_end], stroke);
            t += step;
        }
    };
    let tl = rect.left_top();
    let tr = rect.right_top();
    let br = rect.right_bottom();
    let bl = rect.left_bottom();
    draw_edge(tl, tr);
    draw_edge(tr, br);
    draw_edge(br, bl);
    draw_edge(bl, tl);
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

fn layout_card_title(
    ui: &egui::Ui,
    text: &str,
    wrap_width: f32,
    color: Color32,
) -> std::sync::Arc<egui::Galley> {
    let display = if text.trim().is_empty() {
        "—"
    } else {
        text.trim()
    };
    let mut job = egui::text::LayoutJob::simple(
        display.to_owned(),
        FontId::proportional(12.5_f32),
        color,
        wrap_width.max(24.0_f32),
    );
    job.wrap.max_rows = 6;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.fonts_mut(|f| f.layout_job(job))
}

/// Native `egui` 4-column Kanban Task Board (`TaskBoard.vue` parity).
#[derive(Debug, Default, Clone)]
pub struct TasksBoard {
    draft: String,
    focused_task_id: Option<u64>,
    last_ordered_ids: Vec<u64>,
    dragging_task_id: Option<u64>,
    focus_input_requested: bool,
    input_widget_id: Option<Id>,
    held_key: Option<Key>,
}

impl TasksBoard {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears the creation draft and resets transient focus, drag, and key-repeat state.
    pub fn reset(&mut self) {
        self.draft.clear();
        self.focused_task_id = None;
        self.last_ordered_ids.clear();
        self.dragging_task_id = None;
        self.focus_input_requested = false;
        self.held_key = None;
    }

    /// Returns the current bounded creation draft.
    #[must_use]
    pub fn draft(&self) -> &str {
        &self.draft
    }

    /// Sets the creation draft, clamped to [`MAX_TASK_TITLE_CHARS`] (`1000` chars).
    pub fn set_draft(&mut self, draft: impl Into<String>) {
        self.draft = draft.into().chars().take(MAX_TASK_TITLE_CHARS).collect();
    }

    /// Requests keyboard focus on the `"Add a task…"` input on the next frame.
    pub fn focus_input(&mut self, ctx: &egui::Context) {
        self.focus_input_requested = true;
        if let Some(id) = self.input_widget_id {
            ctx.memory_mut(|m| m.request_focus(id));
        }
        ctx.request_repaint();
    }

    fn ordered_tasks(tasks: &[TaskCard]) -> [Vec<&TaskCard>; TASK_COLUMN_COUNT] {
        let mut buckets: [Vec<&TaskCard>; TASK_COLUMN_COUNT] =
            [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for task in tasks {
            buckets[clamp_column(task.column)].push(task);
        }
        buckets
    }

    fn flat_ordered_ids(buckets: &[Vec<&TaskCard>; TASK_COLUMN_COUNT]) -> Vec<u64> {
        let mut ids = Vec::new();
        for col in buckets {
            for task in col {
                ids.push(task.id);
            }
        }
        ids
    }

    fn classify_focus(
        focused: Option<Id>,
        board_id: Id,
        can_submit: bool,
        has_tasks: bool,
    ) -> Option<BoardFocusOwner> {
        let fid = focused?;
        if fid == board_id.with("add-input") {
            Some(BoardFocusOwner::Input)
        } else if can_submit && fid == board_id.with("add-btn") {
            Some(BoardFocusOwner::AddButton)
        } else if has_tasks && fid == board_id.with("list") {
            Some(BoardFocusOwner::List)
        } else {
            None
        }
    }

    fn update_key_releases(&mut self, ui: &egui::Ui, keyboard_blocked: bool) {
        if keyboard_blocked || !ui.input(|i| i.focused) {
            self.held_key = None;
            return;
        }
        if let Some(held) = self.held_key {
            let released = ui.input(|i| {
                i.events.iter().any(|ev| {
                    matches!(
                        ev,
                        egui::Event::Key {
                            key,
                            pressed: false,
                            ..
                        } if *key == held
                    )
                })
            });
            if released {
                self.held_key = None;
            }
        }
    }

    fn handle_focused_keys(
        &mut self,
        ui: &mut egui::Ui,
        owner: BoardFocusOwner,
        list_id: Id,
        tasks: &[TaskCard],
        ordered_ids: &[u64],
    ) -> (Option<TasksBoardAction>, bool) {
        let held_before = self.held_key;
        let mut action = None;
        let mut scroll_to_focused = false;

        match owner {
            BoardFocusOwner::Input => {
                let (fresh_enter, still_held) = ui.input_mut(|i| {
                    let fresh = held_before != Some(Key::Enter)
                        && i.events.iter().any(|ev| {
                            matches!(
                                ev,
                                egui::Event::Key {
                                    key: Key::Enter,
                                    pressed: true,
                                    repeat: false,
                                    modifiers,
                                    ..
                                } if modifiers.matches_logically(Modifiers::NONE)
                            )
                        });
                    let still = key_held_after_events(&i.events, Key::Enter);
                    let _ = i.consume_key(Modifiers::NONE, Key::Enter);
                    (fresh, still)
                });
                if fresh_enter {
                    self.held_key = still_held.then_some(Key::Enter);
                    if let Some(title) = sanitized_submit_title(&self.draft) {
                        self.draft.clear();
                        action = Some(TasksBoardAction::Add(title));
                    }
                }
            }
            BoardFocusOwner::AddButton => {
                let mut pressed_key: Option<Key> = None;
                let mut still_held = false;
                ui.input_mut(|i| {
                    for ev in &i.events {
                        if let egui::Event::Key {
                            key: k @ (Key::Enter | Key::Space),
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } = ev
                            && modifiers.matches_logically(Modifiers::NONE)
                            && held_before != Some(*k)
                        {
                            pressed_key = Some(*k);
                            break;
                        }
                    }
                    if let Some(k) = pressed_key {
                        still_held = key_held_after_events(&i.events, k);
                    }
                    let _ = i.consume_key(Modifiers::NONE, Key::Enter);
                    let _ = i.consume_key(Modifiers::NONE, Key::Space);
                });
                if let Some(k) = pressed_key {
                    self.held_key = still_held.then_some(k);
                    if let Some(title) = sanitized_submit_title(&self.draft) {
                        self.draft.clear();
                        action = Some(TasksBoardAction::Add(title));
                    }
                }
            }
            BoardFocusOwner::List => {
                let mut move_dir: Option<(Key, isize)> = None;
                let mut still_held = false;
                let (down, up) = ui.input_mut(|i| {
                    for ev in &i.events {
                        if let egui::Event::Key {
                            key: k @ (Key::ArrowLeft | Key::ArrowRight),
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } = ev
                            && (modifiers.matches_logically(Modifiers::NONE)
                                || modifiers.matches_logically(Modifiers::ALT))
                            && held_before != Some(*k)
                        {
                            let step = if *k == Key::ArrowLeft { -1 } else { 1 };
                            move_dir = Some((*k, step));
                            break;
                        }
                    }
                    if let Some((k, _)) = move_dir {
                        still_held = key_held_after_events(&i.events, k);
                    }
                    let _ = i.consume_key(Modifiers::NONE, Key::ArrowLeft);
                    let _ = i.consume_key(Modifiers::NONE, Key::ArrowRight);
                    let _ = i.consume_key(Modifiers::ALT, Key::ArrowLeft);
                    let _ = i.consume_key(Modifiers::ALT, Key::ArrowRight);
                    let down = i.consume_key(Modifiers::NONE, Key::ArrowDown);
                    let up = i.consume_key(Modifiers::NONE, Key::ArrowUp);
                    (down, up)
                });

                if (down || up) && !ordered_ids.is_empty() {
                    ui.ctx().memory_mut(|m| m.request_focus(list_id));
                    let len = ordered_ids.len() as isize;
                    let step: isize = if down { 1 } else { -1 };
                    let cur_pos = self
                        .focused_task_id
                        .and_then(|id| ordered_ids.iter().position(|&x| x == id));
                    let next_pos = match cur_pos {
                        Some(pos) => (pos as isize + step).rem_euclid(len) as usize,
                        None if down => 0,
                        None => ordered_ids.len() - 1,
                    };
                    self.focused_task_id = Some(ordered_ids[next_pos]);
                    scroll_to_focused = true;
                } else if let Some((k, delta)) = move_dir {
                    ui.ctx().memory_mut(|m| m.request_focus(list_id));
                    self.held_key = still_held.then_some(k);
                    if self.focused_task_id.is_none() {
                        self.focused_task_id = ordered_ids.first().copied();
                    }
                    if let Some(fid) = self.focused_task_id
                        && let Some(card) = tasks.iter().find(|t| t.id == fid)
                    {
                        let cur_col = clamp_column(card.column) as isize;
                        let target_col = cur_col + delta;
                        if (0..TASK_COLUMN_COUNT as isize).contains(&target_col) {
                            action = Some(TasksBoardAction::Move {
                                id: fid,
                                column: target_col as usize,
                            });
                            scroll_to_focused = true;
                        }
                    }
                }
            }
        }

        (action, scroll_to_focused)
    }

    /// Renders the 4-column Kanban Task Board and returns any triggered [`TasksBoardAction`].
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        input: TasksBoardInput<'_>,
    ) -> Option<TasksBoardAction> {
        clamp_draft_in_place(&mut self.draft);

        let board_id = ui.id().with("tessel-tasks-board");
        let add_input_id = board_id.with("add-input");
        let add_btn_id = board_id.with("add-btn");
        let list_id = board_id.with("list");
        self.input_widget_id = Some(add_input_id);

        if !input.project_selected {
            self.dragging_task_id = None;
            self.focus_input_requested = false;
            self.held_key = None;
            let avail_w = ui.available_width().max(80.0_f32);
            ui.add_space(14.0_f32);
            let msg = select_project_label(input.french);
            let (_, rect) = ui.allocate_space(Vec2::new(avail_w, 24.0_f32));
            let inner = rect.shrink2(Vec2::new(12.0_f32, 0.0_f32));
            let resp = ui.interact(rect, board_id.with("no-project"), Sense::hover());
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), msg)
            });
            paint_single_line_ellipsis(
                ui.painter(),
                inner,
                msg,
                FontId::proportional(12.5_f32),
                ui.visuals().weak_text_color(),
            );
            return None;
        }

        let popup_open = egui::Popup::is_any_open(ui.ctx());
        let window_focused = ui.input(|i| i.focused);
        let interaction_blocked = !ui.is_enabled() || !window_focused || popup_open;
        self.update_key_releases(ui, interaction_blocked);

        let buckets = Self::ordered_tasks(input.tasks);
        let ordered_ids = Self::flat_ordered_ids(&buckets);
        let has_tasks = !ordered_ids.is_empty();

        if let Some(cur) = self.focused_task_id
            && !ordered_ids.contains(&cur)
        {
            self.focused_task_id = if has_tasks
                && let Some(old_pos) = self.last_ordered_ids.iter().position(|&id| id == cur)
            {
                Some(ordered_ids[old_pos.min(ordered_ids.len() - 1)])
            } else {
                ordered_ids.first().copied()
            };
        }

        if let Some(drag_id) = self.dragging_task_id
            && (interaction_blocked || !ordered_ids.contains(&drag_id))
        {
            self.dragging_task_id = None;
        }

        if !has_tasks && ui.is_enabled() && ui.ctx().memory(|m| m.focused()) == Some(list_id) {
            ui.ctx().memory_mut(|m| m.request_focus(add_input_id));
        }

        let can_submit_before = sanitized_submit_title(&self.draft).is_some();
        let focused_before = ui.ctx().memory(|m| m.focused());
        let focus_owner = if interaction_blocked {
            if !ui.is_enabled() {
                self.focus_input_requested = false;
            }
            None
        } else {
            let classified =
                Self::classify_focus(focused_before, board_id, can_submit_before, has_tasks);
            if classified == Some(BoardFocusOwner::List) && self.focused_task_id.is_none() {
                self.focused_task_id = ordered_ids.first().copied();
            }
            if classified.is_none() {
                self.held_key = None;
            }
            classified
        };

        let (mut action, scroll_to_focused) = if let Some(owner) = focus_owner {
            self.handle_focused_keys(ui, owner, list_id, input.tasks, &ordered_ids)
        } else {
            (None, false)
        };

        ui.spacing_mut().item_spacing.y = 0.0_f32;

        // 1. `.task-board-add` top creation form (`32px` input + `"Add"` / `"Ajouter"` button).
        if let Some(add_act) = self.render_add_bar(
            ui,
            add_input_id,
            add_btn_id,
            input.french,
            interaction_blocked,
        ) {
            action = Some(add_act);
        }
        clamp_draft_in_place(&mut self.draft);

        // 2. Focusable list container (`Tab` stops once for all task cards).
        let list_resp = if has_tasks {
            let viewport = ui.available_rect_before_wrap();
            Some(ui.interact(viewport, list_id, Sense::focusable_noninteractive()))
        } else {
            None
        };

        let list_has_focus =
            has_tasks && ui.is_enabled() && ui.ctx().memory(|m| m.has_focus(list_id));
        let active_focused_id = if list_has_focus {
            self.focused_task_id
                .or_else(|| ordered_ids.first().copied())
        } else {
            None
        };

        let pointer_pos = ui.input(|i| i.pointer.interact_pos());
        let pointer_released = ui.input(|i| i.pointer.any_released());
        let escape_cancel_drag =
            self.dragging_task_id.is_some() && ui.input(|i| i.key_pressed(Key::Escape));
        if escape_cancel_drag {
            self.dragging_task_id = None;
        }

        let mut column_rects: [Option<Rect>; TASK_COLUMN_COUNT] = [None; TASK_COLUMN_COUNT];

        // 3. `.task-board-columns`: stacked sections in narrow panels (`< 720px`, matching
        // `style.css:1437-1446` with zero forced parent width) and 4 equal columns when wide.
        egui::ScrollArea::vertical()
            .id_salt(board_id.with("scroll"))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let avail_w = ui.available_width().max(100.0_f32);
                let pad_x = if avail_w < 240.0_f32 {
                    6.0_f32
                } else {
                    12.0_f32
                };
                ui.add_space(6.0_f32);

                let inner_w = (avail_w - pad_x * 2.0_f32).max(80.0_f32);
                ui.horizontal(|ui| {
                    ui.add_space(pad_x);
                    ui.vertical(|ui| {
                        ui.set_max_width(inner_w);
                        if inner_w >= 720.0_f32 {
                            ui.columns(TASK_COLUMN_COUNT, |cols| {
                                for col_idx in 0..TASK_COLUMN_COUNT {
                                    let (col_act, rect) = self.render_column(
                                        &mut cols[col_idx],
                                        board_id,
                                        list_id,
                                        col_idx,
                                        &buckets[col_idx],
                                        input.french,
                                        active_focused_id,
                                        scroll_to_focused,
                                        pointer_pos,
                                        interaction_blocked,
                                    );
                                    column_rects[col_idx] = Some(rect);
                                    if col_act.is_some() {
                                        action = col_act;
                                    }
                                }
                            });
                        } else {
                            ui.spacing_mut().item_spacing.y = 10.0_f32;
                            for col_idx in 0..TASK_COLUMN_COUNT {
                                let (col_act, rect) = self.render_column(
                                    ui,
                                    board_id,
                                    list_id,
                                    col_idx,
                                    &buckets[col_idx],
                                    input.french,
                                    active_focused_id,
                                    scroll_to_focused,
                                    pointer_pos,
                                    interaction_blocked,
                                );
                                column_rects[col_idx] = Some(rect);
                                if col_act.is_some() {
                                    action = col_act;
                                }
                            }
                        }
                    });
                });
                ui.add_space(12.0_f32);
            });

        // Complete drag-and-drop move if pointer released over a target column.
        if pointer_released
            && let Some(drag_id) = self.dragging_task_id.take()
            && !interaction_blocked
            && let Some(pos) = pointer_pos
            && let Some(card) = input.tasks.iter().find(|t| t.id == drag_id)
        {
            let src_col = clamp_column(card.column);
            for (target_col, rect_opt) in column_rects.iter().enumerate() {
                if let Some(r) = rect_opt
                    && r.contains(pos)
                    && target_col != src_col
                {
                    self.focused_task_id = Some(drag_id);
                    action = Some(TasksBoardAction::Move {
                        id: drag_id,
                        column: target_col,
                    });
                    break;
                }
            }
        }

        if let Some(list_resp) = list_resp {
            let active_card = self
                .focused_task_id
                .and_then(|id| input.tasks.iter().find(|t| t.id == id))
                .or_else(|| {
                    ordered_ids
                        .first()
                        .and_then(|id| input.tasks.iter().find(|t| t.id == *id))
                });
            let landmark = board_landmark_label(input.french);
            let list_a11y = match active_card {
                Some(c) => format!(
                    "{landmark} — {} ({})",
                    c.title.trim(),
                    column_label(c.column, input.french)
                ),
                None => landmark.to_owned(),
            };
            list_resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), &list_a11y)
            });
        }

        let can_submit_after = sanitized_submit_title(&self.draft).is_some();
        if !interaction_blocked {
            ui.ctx().memory_mut(|m| {
                m.set_focus_lock_filter(
                    add_input_id,
                    egui::EventFilter {
                        tab: false,
                        horizontal_arrows: true,
                        vertical_arrows: false,
                        escape: false,
                    },
                );
                if can_submit_after {
                    m.set_focus_lock_filter(
                        add_btn_id,
                        egui::EventFilter {
                            tab: false,
                            horizontal_arrows: true,
                            vertical_arrows: false,
                            escape: false,
                        },
                    );
                }
                if has_tasks {
                    m.set_focus_lock_filter(
                        list_id,
                        egui::EventFilter {
                            tab: false,
                            horizontal_arrows: true,
                            vertical_arrows: true,
                            escape: false,
                        },
                    );
                }
            });
        }
        self.last_ordered_ids = ordered_ids;

        action
    }

    fn render_add_bar(
        &mut self,
        ui: &mut egui::Ui,
        add_input_id: Id,
        add_btn_id: Id,
        french: bool,
        interaction_blocked: bool,
    ) -> Option<TasksBoardAction> {
        let avail_w = ui.available_width().max(80.0_f32);
        let pad_x = if avail_w < 240.0_f32 {
            8.0_f32
        } else {
            12.0_f32
        };
        let pad_y = 10.0_f32;
        let bar_h = 32.0_f32 + pad_y * 2.0_f32;
        let (_, bar_rect) = ui.allocate_space(Vec2::new(avail_w, bar_h));

        let border_col = ui.visuals().widgets.noninteractive.bg_stroke.color;
        ui.painter().line_segment(
            [
                Pos2::new(bar_rect.min.x, bar_rect.max.y - 0.5_f32),
                Pos2::new(bar_rect.max.x, bar_rect.max.y - 0.5_f32),
            ],
            Stroke::new(1.0_f32, border_col),
        );

        let btn_label = add_button_label(french);
        let btn_font = FontId::proportional(12.0_f32);
        let btn_text_w = ui
            .fonts_mut(|f| {
                f.layout_no_wrap(btn_label.to_owned(), btn_font.clone(), Color32::WHITE)
                    .size()
                    .x
            })
            .ceil();
        let btn_w = (btn_text_w + 24.0_f32).clamp(52.0_f32, 92.0_f32);

        let row_min_x = bar_rect.min.x + pad_x;
        let row_max_x = (bar_rect.max.x - pad_x).max(row_min_x + 68.0_f32);
        let row_y = bar_rect.min.y + pad_y;

        let btn_rect = Rect::from_min_size(
            Pos2::new((row_max_x - btn_w).max(row_min_x + 36.0_f32), row_y),
            Vec2::new(btn_w, 32.0_f32),
        );
        let input_box_rect = Rect::from_min_max(
            Pos2::new(row_min_x, row_y),
            Pos2::new(
                (btn_rect.min.x - 6.0_f32).max(row_min_x + 28.0_f32),
                row_y + 32.0_f32,
            ),
        );

        let focus_req = self.focus_input_requested;
        self.focus_input_requested = false;
        let input_focused =
            ui.is_enabled() && (focus_req || ui.ctx().memory(|m| m.has_focus(add_input_id)));

        let input_stroke = if input_focused {
            Stroke::new(1.0_f32, ui.visuals().selection.stroke.color)
        } else {
            Stroke::new(
                1.0_f32,
                ui.visuals()
                    .widgets
                    .inactive
                    .bg_stroke
                    .color
                    .gamma_multiply(0.9_f32),
            )
        };
        ui.painter().with_clip_rect(input_box_rect).rect(
            input_box_rect,
            6.0_f32,
            ui.visuals().extreme_bg_color,
            input_stroke,
            StrokeKind::Inside,
        );

        let text_inner = input_box_rect.shrink2(Vec2::new(9.0_f32, 4.0_f32));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text_inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.set_clip_rect(ui.clip_rect().intersect(text_inner));
        let placeholder = add_placeholder(french);
        let edit_out = egui::TextEdit::singleline(&mut self.draft)
            .id(add_input_id)
            .frame(false)
            .lock_focus(false)
            .font(FontId::proportional(12.5_f32))
            .hint_text(placeholder)
            .char_limit(MAX_TASK_TITLE_CHARS)
            .desired_width(text_inner.width().max(8.0_f32))
            .show(&mut child);
        if focus_req && ui.is_enabled() && !ui.ctx().memory(|m| m.has_focus(add_input_id)) {
            edit_out.response.request_focus();
        }
        edit_out.response.on_hover_text(placeholder);

        let can_submit = ui.is_enabled() && sanitized_submit_title(&self.draft).is_some();
        let btn_sense = if can_submit {
            Sense::click()
        } else {
            Sense::CLICK
        };
        let btn_resp = ui.interact(btn_rect, add_btn_id, btn_sense);
        btn_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, can_submit, btn_label)
        });
        let btn_focused = can_submit && ui.ctx().memory(|m| m.has_focus(add_btn_id));
        let accent = ui.visuals().selection.stroke.color;
        let btn_stroke = if can_submit && (btn_resp.hovered() || btn_focused) {
            Stroke::new(1.0_f32, accent)
        } else {
            Stroke::new(
                1.0_f32,
                border_col.gamma_multiply(if can_submit { 1.0_f32 } else { 0.55_f32 }),
            )
        };
        let btn_bg = if can_submit && btn_resp.hovered() {
            ui.visuals()
                .widgets
                .hovered
                .bg_fill
                .gamma_multiply(0.45_f32)
        } else {
            Color32::TRANSPARENT
        };
        let btn_fg = if !can_submit {
            ui.visuals().weak_text_color().gamma_multiply(0.55_f32)
        } else if btn_resp.hovered() || btn_focused {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        let bp = ui.painter().with_clip_rect(btn_rect);
        bp.rect(btn_rect, 6.0_f32, btn_bg, btn_stroke, StrokeKind::Inside);
        let btn_galley = bp.layout_no_wrap(btn_label.to_owned(), btn_font, btn_fg);
        bp.galley(
            Pos2::new(
                btn_rect.center().x - btn_galley.size().x * 0.5_f32,
                btn_rect.center().y - btn_galley.size().y * 0.5_f32,
            ),
            btn_galley,
            btn_fg,
        );
        let btn_clicked = can_submit && !interaction_blocked && btn_resp.clicked();
        btn_resp.on_hover_text(btn_label);

        if btn_clicked && let Some(title) = sanitized_submit_title(&self.draft) {
            self.draft.clear();
            ui.ctx().memory_mut(|m| m.request_focus(add_input_id));
            Some(TasksBoardAction::Add(title))
        } else {
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_column(
        &mut self,
        ui: &mut egui::Ui,
        board_id: Id,
        list_id: Id,
        col_idx: usize,
        cards: &[&TaskCard],
        french: bool,
        active_focused_id: Option<u64>,
        scroll_to_focused: bool,
        pointer_pos: Option<Pos2>,
        interaction_blocked: bool,
    ) -> (Option<TasksBoardAction>, Rect) {
        let col_color = column_color(col_idx);
        let col_title = column_label(col_idx, french);
        let full_w = ui.available_width().max(64.0_f32);
        let mut action = None;

        let col_resp = ui.vertical(|ui| {
            ui.set_max_width(full_w);
            ui.spacing_mut().item_spacing.y = 6.0_f32;

            // `.task-column-head`: 8px colored dot + bold 12px title + count pill.
            let (_, head_rect) = ui.allocate_space(Vec2::new(full_w, 24.0_f32));
            let hp = ui.painter().with_clip_rect(head_rect);
            let dot_center = Pos2::new(head_rect.min.x + 6.0_f32, head_rect.center().y);
            hp.circle_filled(dot_center, 4.0_f32, col_color);

            let count_str = cards.len().to_string();
            let count_fg = ui.visuals().weak_text_color();
            let count_galley =
                hp.layout_no_wrap(count_str, FontId::proportional(10.5_f32), count_fg);
            let pill_w = (count_galley.size().x + 12.0_f32).max(18.0_f32);

            let title_min_x = head_rect.min.x + 16.0_f32;
            let max_title_w = (head_rect.max.x - title_min_x - pill_w - 8.0_f32).max(24.0_f32);
            let mut title_job = egui::text::LayoutJob::simple(
                col_title.to_owned(),
                FontId::proportional(12.0_f32),
                ui.visuals().strong_text_color(),
                max_title_w,
            );
            title_job.wrap.max_rows = 1;
            title_job.wrap.break_anywhere = true;
            title_job.wrap.overflow_character = Some('…');
            let title_galley = hp.layout_job(title_job);
            let title_pos = Pos2::new(
                title_min_x,
                head_rect.center().y - title_galley.size().y * 0.5_f32,
            );
            let pill_x =
                (title_pos.x + title_galley.size().x + 8.0_f32).min(head_rect.max.x - pill_w);
            hp.galley(title_pos, title_galley, ui.visuals().strong_text_color());

            let pill_rect = Rect::from_center_size(
                Pos2::new(pill_x + pill_w * 0.5_f32, head_rect.center().y),
                Vec2::new(pill_w, 17.0_f32),
            );
            hp.rect_filled(
                pill_rect,
                8.5_f32,
                ui.visuals().strong_text_color().gamma_multiply(0.08_f32),
            );
            hp.galley(
                Pos2::new(
                    pill_rect.center().x - count_galley.size().x * 0.5_f32,
                    pill_rect.center().y - count_galley.size().y * 0.5_f32,
                ),
                count_galley,
                count_fg,
            );

            if cards.is_empty() {
                // `.task-column-empty`: dashed rounded box with localized `"No tasks"`.
                let (_, empty_rect) = ui.allocate_space(Vec2::new(full_w, 36.0_f32));
                let drop_hover = self.dragging_task_id.is_some()
                    && pointer_pos.is_some_and(|p| empty_rect.expand(6.0_f32).contains(p));
                let ep = ui.painter().with_clip_rect(empty_rect);
                if drop_hover {
                    ep.rect_filled(empty_rect, 6.0_f32, col_color.gamma_multiply(0.12_f32));
                }
                let dash_color = if drop_hover {
                    col_color.gamma_multiply(0.75_f32)
                } else {
                    ui.visuals()
                        .widgets
                        .noninteractive
                        .bg_stroke
                        .color
                        .gamma_multiply(0.85_f32)
                };
                paint_dashed_rect(
                    &ep,
                    empty_rect.shrink(0.5_f32),
                    Stroke::new(1.0_f32, dash_color),
                    4.0_f32,
                    3.0_f32,
                );
                let empty_lbl = empty_column_label(french);
                let empty_fg = if drop_hover {
                    col_color
                } else {
                    ui.visuals().weak_text_color()
                };
                let eg = ep.layout_no_wrap(
                    empty_lbl.to_owned(),
                    FontId::proportional(11.5_f32),
                    empty_fg,
                );
                ep.galley(
                    Pos2::new(
                        (empty_rect.center().x - eg.size().x * 0.5_f32)
                            .max(empty_rect.min.x + 4.0_f32),
                        empty_rect.center().y - eg.size().y * 0.5_f32,
                    ),
                    eg,
                    empty_fg,
                );
            } else {
                for card in cards {
                    if let Some(card_act) = self.render_card(
                        ui,
                        board_id,
                        list_id,
                        col_idx,
                        col_color,
                        card,
                        french,
                        active_focused_id == Some(card.id),
                        scroll_to_focused,
                        interaction_blocked,
                    ) {
                        action = Some(card_act);
                    }
                }
            }
        });

        // Subtle `.drop-here` highlight on non-empty column body while dragging a card from another column.
        if let Some(drag_id) = self.dragging_task_id
            && !interaction_blocked
            && !cards.is_empty()
            && !cards.iter().any(|c| c.id == drag_id)
            && pointer_pos.is_some_and(|p| col_resp.response.rect.contains(p))
        {
            paint_dashed_rect(
                &ui.painter().with_clip_rect(col_resp.response.rect),
                col_resp.response.rect.shrink(0.5_f32),
                Stroke::new(1.0_f32, col_color.gamma_multiply(0.7_f32)),
                4.0_f32,
                3.0_f32,
            );
        }

        (action, col_resp.response.rect)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_card(
        &mut self,
        ui: &mut egui::Ui,
        board_id: Id,
        list_id: Id,
        col_idx: usize,
        col_color: Color32,
        card: &TaskCard,
        french: bool,
        is_key_focused: bool,
        scroll_to_focused: bool,
        interaction_blocked: bool,
    ) -> Option<TasksBoardAction> {
        let full_w = ui.available_width().max(64.0_f32);
        let text_wrap_w = (full_w - 22.0_f32).max(24.0_f32);
        let fg = ui.visuals().strong_text_color();
        let title_galley = layout_card_title(ui, &card.title, text_wrap_w, fg);

        let title_h = title_galley.size().y.max(16.0_f32);
        let foot_h = 24.0_f32;
        let card_h = 10.0_f32 + title_h + 8.0_f32 + foot_h + 8.0_f32;
        let (_, card_rect) = ui.allocate_space(Vec2::new(full_w, card_h));

        let foot_y = card_rect.max.y - 8.0_f32 - foot_h;
        let right_btn_rect = Rect::from_min_size(
            Pos2::new(card_rect.max.x - 8.0_f32 - 26.0_f32, foot_y),
            Vec2::new(26.0_f32, foot_h),
        );
        let left_btn_rect = Rect::from_min_size(
            Pos2::new(right_btn_rect.min.x - 4.0_f32 - 26.0_f32, foot_y),
            Vec2::new(26.0_f32, foot_h),
        );

        let card_id = board_id.with(("task-card", card.id));
        let left_id = board_id.with(("task-move-left", card.id));
        let right_id = board_id.with(("task-move-right", card.id));

        let can_left = ui.is_enabled() && col_idx > 0;
        let can_right = ui.is_enabled() && col_idx + 1 < TASK_COLUMN_COUNT;

        // Register card body first so move buttons on top win pointer hit-testing cleanly.
        let card_resp = ui.interact(card_rect, card_id, Sense::CLICK | Sense::DRAG);
        let card_a11y = format!("{} ({})", card.title.trim(), column_label(col_idx, french));
        card_resp.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                ui.is_enabled(),
                is_key_focused,
                &card_a11y,
            )
        });
        if scroll_to_focused && is_key_focused {
            card_resp.scroll_to_me(Some(egui::Align::Center));
        }

        let left_label = if col_idx > 0 {
            move_button_label(col_idx - 1, french)
        } else {
            move_button_label(0, french)
        };
        let left_resp = ui.interact(
            left_btn_rect,
            left_id,
            if can_left {
                Sense::CLICK
            } else {
                Sense::hover()
            },
        );
        left_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, can_left, &left_label)
        });

        let right_label = if col_idx + 1 < TASK_COLUMN_COUNT {
            move_button_label(col_idx + 1, french)
        } else {
            move_button_label(TASK_COLUMN_COUNT - 1, french)
        };
        let right_resp = ui.interact(
            right_btn_rect,
            right_id,
            if can_right {
                Sense::CLICK
            } else {
                Sense::hover()
            },
        );
        right_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, can_right, &right_label)
        });

        if !interaction_blocked && card_resp.drag_started() {
            self.dragging_task_id = Some(card.id);
            self.focused_task_id = Some(card.id);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
        }

        let is_dragging = self.dragging_task_id == Some(card.id);
        let cp = ui.painter().with_clip_rect(card_rect);
        let accent = ui.visuals().selection.stroke.color;

        let base_bg = ui.visuals().widgets.noninteractive.bg_fill;
        let bg = if is_dragging {
            base_bg.gamma_multiply(0.5_f32)
        } else if is_key_focused {
            ui.visuals()
                .widgets
                .hovered
                .bg_fill
                .gamma_multiply(0.55_f32)
        } else {
            base_bg
        };
        let border_stroke = if is_key_focused {
            Stroke::new(1.5_f32, accent)
        } else if card_resp.hovered() || left_resp.hovered() || right_resp.hovered() {
            Stroke::new(1.0_f32, ui.visuals().widgets.hovered.bg_stroke.color)
        } else {
            Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color)
        };
        cp.rect(card_rect, 6.0_f32, bg, border_stroke, StrokeKind::Inside);

        // `.task-card { border-left: 3px solid var(--col); }` (`style.css:1599`).
        let stripe_rect = Rect::from_min_max(
            Pos2::new(card_rect.min.x + 0.5_f32, card_rect.min.y + 3.0_f32),
            Pos2::new(card_rect.min.x + 3.5_f32, card_rect.max.y - 3.0_f32),
        );
        cp.rect_filled(stripe_rect, 1.5_f32, col_color);

        cp.galley(
            Pos2::new(card_rect.min.x + 11.0_f32, card_rect.min.y + 10.0_f32),
            title_galley,
            if is_dragging {
                fg.gamma_multiply(0.55_f32)
            } else {
                fg
            },
        );

        // Paint `.task-btn` vector move buttons.
        Self::paint_move_btn(&cp, ui, left_btn_rect, can_left, left_resp.hovered(), true);
        Self::paint_move_btn(
            &cp,
            ui,
            right_btn_rect,
            can_right,
            right_resp.hovered(),
            false,
        );

        let left_clicked = !interaction_blocked && can_left && left_resp.clicked();
        let right_clicked = !interaction_blocked && can_right && right_resp.clicked();
        let card_clicked =
            !interaction_blocked && card_resp.clicked() && !left_clicked && !right_clicked;

        if can_left {
            left_resp.on_hover_text(&left_label);
        }
        if can_right {
            right_resp.on_hover_text(&right_label);
        }

        if left_clicked {
            self.focused_task_id = Some(card.id);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
            Some(TasksBoardAction::Move {
                id: card.id,
                column: col_idx - 1,
            })
        } else if right_clicked {
            self.focused_task_id = Some(card.id);
            ui.ctx().memory_mut(|m| m.request_focus(list_id));
            Some(TasksBoardAction::Move {
                id: card.id,
                column: col_idx + 1,
            })
        } else {
            if card_clicked {
                self.focused_task_id = Some(card.id);
                ui.ctx().memory_mut(|m| m.request_focus(list_id));
            }
            None
        }
    }

    fn paint_move_btn(
        painter: &Painter,
        ui: &egui::Ui,
        rect: Rect,
        enabled: bool,
        hovered: bool,
        left: bool,
    ) {
        let bg = if enabled && hovered {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            Color32::TRANSPARENT
        };
        let border_col = ui
            .visuals()
            .widgets
            .noninteractive
            .bg_stroke
            .color
            .gamma_multiply(if enabled { 1.0_f32 } else { 0.4_f32 });
        painter.rect(
            rect,
            5.0_f32,
            bg,
            Stroke::new(1.0_f32, border_col),
            StrokeKind::Inside,
        );
        let fg = if !enabled {
            ui.visuals().weak_text_color().gamma_multiply(0.35_f32)
        } else if hovered {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(12.0_f32));
        paint_chevron_horizontal(painter, icon_rect, fg, left);
    }
}
