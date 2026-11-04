//! Pure composite scrollback, cell lookup, and copy extraction over archived
//! pre-resync [`History`] rows and the live [`vt100::Parser`] screen.
//!
//! No terminal replies, network operations, or input bytes are ever produced here:
//! archived rows are read strictly as already-decoded [`vt100::Cell`] values.
use super::host_history::{History, MAX_ROWS};

/// Number of scrollback lines currently stored in `parser` (`0` on the alternate screen),
/// restoring `parser`'s current scrollback offset before returning.
#[must_use]
pub fn live_scrollback_len(parser: &mut vt100::Parser) -> usize {
    let current = parser.screen().scrollback();
    parser.set_scrollback(usize::MAX);
    let max = parser.screen().scrollback();
    parser.set_scrollback(current);
    max
}

/// Monotonic scroll state bridging archived pre-resync [`History`] rows above the
/// live [`vt100::Parser`] scrollback and visible screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollState {
    history_offset: usize,
    pinned_row_id: Option<u64>,
}

/// Snapshot of scroll coordinates captured immediately before `Feed::poll`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollAnchor {
    history_offset: usize,
    pinned_row_id: Option<u64>,
    old_total_captured: u64,
    old_live_scrollback: usize,
    old_live_max: usize,
    old_size: (u16, u16),
}

/// Outcome of reconciling [`ScrollState`] after `Feed::poll`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollOutcome {
    /// True when rows currently visible in the viewport may have changed or been evicted,
    /// indicating that any active text selection should be cleared.
    pub clear_selection: bool,
}

impl ScrollState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            history_offset: 0,
            pinned_row_id: None,
        }
    }

    /// Number of rows scrolled into archived pre-resync [`History`] above the live parser
    /// (`0` when viewing only the live parser's screen or live scrollback).
    #[must_use]
    pub const fn history_offset(&self) -> usize {
        self.history_offset
    }

    /// True when at least the top row of the viewport displays archived pre-resync [`History`].
    #[must_use]
    pub const fn in_archived_history(&self) -> bool {
        self.history_offset > 0
    }

    /// True when scrolled into either archived [`History`] or live parser scrollback.
    #[must_use]
    pub fn viewing_scrollback(&self, parser: &vt100::Parser) -> bool {
        self.history_offset > 0 || parser.screen().scrollback() > 0
    }

    /// Combined scrollback distance from the live screen bottom (`history_offset + parser.scrollback()`).
    #[must_use]
    pub fn total_scrollback(&self, parser: &vt100::Parser) -> usize {
        self.history_offset
            .saturating_add(parser.screen().scrollback())
    }

    /// Pure input gate: typing, paste, IME, and interrupt keys remain enabled on the live screen
    /// and live parser scrollback (which snaps back to `0` on input), and are strictly blocked
    /// while viewing archived pre-resync [`History`] rows.
    #[must_use]
    pub const fn accepts_live_input(&self, pane_accepts_input: bool) -> bool {
        pane_accepts_input && !self.in_archived_history()
    }

    /// Pure mouse-reporting gate: disabled whenever viewing scrollback, archived [`History`],
    /// or an in-pane overlay.
    #[must_use]
    pub fn allows_mouse_reporting(
        &self,
        pane_accepts_input: bool,
        overlay_pointer: bool,
        parser: &vt100::Parser,
    ) -> bool {
        self.accepts_live_input(pane_accepts_input)
            && !overlay_pointer
            && !self.viewing_scrollback(parser)
    }

    /// Pure cursor/IME visibility gate: shown only on the live bottom screen (`!viewing_scrollback`).
    #[must_use]
    pub fn shows_cursor(&self, pane_accepts_input: bool, parser: &vt100::Parser) -> bool {
        self.accepts_live_input(pane_accepts_input)
            && !parser.screen().hide_cursor()
            && !self.viewing_scrollback(parser)
    }

    /// Reset both archived [`History`] offset and live `parser` scrollback to `0`
    /// (`"Back to live output"`). Returns `true` if the viewport moved.
    pub fn reset_to_live(&mut self, parser: &mut vt100::Parser) -> bool {
        let was_scrolled = self.history_offset > 0 || parser.screen().scrollback() > 0;
        self.history_offset = 0;
        self.pinned_row_id = None;
        parser.set_scrollback(0);
        was_scrolled
    }

    /// Re-clamp `history_offset` and keep `parser` scrollback aligned with `live_scrollback_len`
    /// whenever viewing archived [`History`].
    pub fn sync(&mut self, history: Option<&History>, parser: &mut vt100::Parser) {
        let Some(history) = history.filter(|h| !h.is_empty()) else {
            self.history_offset = 0;
            self.pinned_row_id = None;
            return;
        };
        if self.history_offset == 0 {
            self.pinned_row_id = None;
            return;
        }
        let live_max = live_scrollback_len(parser);
        parser.set_scrollback(live_max);
        let evicted = history.evicted_rows();
        if let Some(pinned_id) = self.pinned_row_id {
            if pinned_id >= evicted {
                let idx = usize::try_from(pinned_id - evicted)
                    .unwrap_or(usize::MAX)
                    .min(history.len() - 1);
                self.history_offset = history.len() - idx;
                self.pinned_row_id = Some(evicted.saturating_add(idx as u64));
            } else {
                self.history_offset = history.len();
                self.pinned_row_id = Some(evicted);
            }
        } else {
            self.history_offset = self.history_offset.min(history.len());
            let idx = history.len() - self.history_offset;
            self.pinned_row_id = Some(evicted.saturating_add(idx as u64));
        }
    }

    /// Scroll by `delta_lines` (`> 0` up toward older history, `< 0` down toward live output)
    /// without integer overflow across `i64::MIN..=i64::MAX`.
    ///
    /// Returns `true` when the effective scroll position changed.
    pub fn scroll_by(
        &mut self,
        delta_lines: i64,
        history: Option<&History>,
        parser: &mut vt100::Parser,
    ) -> bool {
        self.sync(history, parser);
        let archived_len = history.map_or(0, History::len);
        let live_max = live_scrollback_len(parser);
        let before_live = parser.screen().scrollback();
        let before_hist = self.history_offset;
        let current = if before_hist > 0 {
            live_max.saturating_add(before_hist)
        } else {
            before_live
        };
        let max_offset = live_max.saturating_add(archived_len);
        let target = if delta_lines >= 0 {
            let add = usize::try_from(delta_lines).unwrap_or(usize::MAX);
            current.saturating_add(add).min(max_offset)
        } else {
            let sub = usize::try_from(delta_lines.unsigned_abs()).unwrap_or(usize::MAX);
            current.saturating_sub(sub).min(max_offset)
        };
        if target <= live_max {
            self.history_offset = 0;
            self.pinned_row_id = None;
            parser.set_scrollback(target);
        } else {
            let new_hist = (target - live_max).min(archived_len);
            self.history_offset = new_hist;
            parser.set_scrollback(live_max);
            if let Some(history) = history {
                let idx = archived_len - new_hist;
                self.pinned_row_id = Some(history.evicted_rows().saturating_add(idx as u64));
            }
        }
        self.history_offset != before_hist || parser.screen().scrollback() != before_live
    }

    /// Capture scroll anchor immediately before `Feed::poll` mutates `history` and `parser`.
    pub fn before_poll(&mut self, history: &History, parser: &mut vt100::Parser) -> PollAnchor {
        self.sync(Some(history), parser);
        let old_live_max = live_scrollback_len(parser);
        PollAnchor {
            history_offset: self.history_offset,
            pinned_row_id: self.pinned_row_id,
            old_total_captured: history.total_captured_rows(),
            old_live_scrollback: parser.screen().scrollback(),
            old_live_max,
            old_size: parser.screen().size(),
        }
    }

    /// Reconcile scroll position after `Feed::poll` without jumping when reading archived
    /// [`History`] or scrollback across live output and resync snapshots.
    pub fn after_poll(
        &mut self,
        anchor: PollAnchor,
        history: &History,
        parser: &mut vt100::Parser,
        applied_changed: bool,
    ) -> PollOutcome {
        let new_size = parser.screen().size();
        let size_changed = new_size != anchor.old_size;
        let new_total_captured = history.total_captured_rows();
        let snapshot_captured = new_total_captured > anchor.old_total_captured;

        if anchor.history_offset == 0 && anchor.old_live_scrollback == 0 {
            self.history_offset = 0;
            self.pinned_row_id = None;
            parser.set_scrollback(0);
            return PollOutcome {
                clear_selection: applied_changed || size_changed || snapshot_captured,
            };
        }

        if anchor.history_offset > 0 {
            if history.is_empty() {
                self.history_offset = 0;
                self.pinned_row_id = None;
                parser.set_scrollback(0);
                return PollOutcome {
                    clear_selection: true,
                };
            }
            let new_live_max = live_scrollback_len(parser);
            parser.set_scrollback(new_live_max);
            let evicted = history.evicted_rows();
            let pinned_id = anchor.pinned_row_id.unwrap_or(evicted);
            if pinned_id < evicted {
                self.history_offset = history.len();
                self.pinned_row_id = Some(evicted);
                return PollOutcome {
                    clear_selection: true,
                };
            }
            let idx = usize::try_from(pinned_id - evicted)
                .unwrap_or(usize::MAX)
                .min(history.len() - 1);
            self.history_offset = history.len() - idx;
            self.pinned_row_id = Some(evicted.saturating_add(idx as u64));

            let straddles_live_seam = self.history_offset < usize::from(new_size.0)
                || anchor.history_offset < usize::from(anchor.old_size.0);
            let seam_mutated = straddles_live_seam
                && (snapshot_captured
                    || applied_changed
                        && (anchor.old_live_max < usize::from(new_size.0)
                            || new_live_max >= MAX_ROWS));
            return PollOutcome {
                clear_selection: size_changed || seam_mutated,
            };
        }

        if !snapshot_captured {
            self.history_offset = 0;
            self.pinned_row_id = None;
            let new_live_scrollback = parser.screen().scrollback();
            let hit_live_cap = applied_changed && new_live_scrollback >= MAX_ROWS;
            let visible_live_drawn =
                applied_changed && new_live_scrollback < usize::from(new_size.0);
            return PollOutcome {
                clear_selection: size_changed || hit_live_cap || visible_live_drawn,
            };
        }

        if history.is_empty() {
            self.history_offset = 0;
            self.pinned_row_id = None;
            parser.set_scrollback(0);
            return PollOutcome {
                clear_selection: true,
            };
        }
        let new_live_max = live_scrollback_len(parser);
        parser.set_scrollback(new_live_max);
        let evicted = history.evicted_rows();
        let Some(origin) = history.capture_origin(anchor.old_total_captured) else {
            // Later snapshots in this poll evicted the entire first capture.
            self.history_offset = history.len();
            self.pinned_row_id = Some(evicted);
            return PollOutcome {
                clear_selection: true,
            };
        };
        let old_top_from_start = origin.visible_top as u64;
        let skipped_front = origin.skipped_front as u64;
        let source_view_reset = anchor.old_live_scrollback > 0 && origin.scrollback == 0;
        let source_truncated = old_top_from_start < skipped_front;
        let target_id = anchor
            .old_total_captured
            .saturating_add(old_top_from_start.saturating_sub(skipped_front));
        if target_id < evicted {
            self.history_offset = history.len();
            self.pinned_row_id = Some(evicted);
            return PollOutcome {
                clear_selection: true,
            };
        }
        let idx = usize::try_from(target_id - evicted)
            .unwrap_or(usize::MAX)
            .min(history.len() - 1);
        self.history_offset = history.len() - idx;
        self.pinned_row_id = Some(evicted.saturating_add(idx as u64));
        let straddles_live_seam = self.history_offset < usize::from(new_size.0);
        let old_straddled_live = anchor.old_live_scrollback < usize::from(anchor.old_size.0);
        PollOutcome {
            clear_selection: size_changed
                || source_view_reset
                || source_truncated
                || straddles_live_seam
                || (applied_changed && old_straddled_live),
        }
    }
}

/// Borrowed, zero-allocation composite screen view over archived [`History`] + live [`vt100::Screen`].
#[derive(Clone, Copy)]
pub struct Viewport<'a> {
    history: Option<&'a History>,
    screen: &'a vt100::Screen,
    rows: u16,
    cols: u16,
    history_offset: usize,
    history_top: usize,
}

impl<'a> Viewport<'a> {
    #[must_use]
    pub fn new(
        history: Option<&'a History>,
        screen: &'a vt100::Screen,
        scroll: &ScrollState,
    ) -> Self {
        let (rows, cols) = screen.size();
        let archived_len = history.map_or(0, History::len);
        let history_offset = scroll.history_offset.min(archived_len);
        let history_top = archived_len - history_offset;
        Self {
            history,
            screen,
            rows,
            cols,
            history_offset,
            history_top,
        }
    }

    #[must_use]
    pub const fn size(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }

    /// Number of visible rows (`0..=rows`) sourced from archived [`History`].
    #[must_use]
    pub fn history_rows_shown(&self) -> u16 {
        self.history_offset.min(usize::from(self.rows)) as u16
    }

    #[must_use]
    pub const fn in_archived_history(&self) -> bool {
        self.history_offset > 0
    }

    /// Borrow the cell at `(row, col)` within `(0..rows, 0..cols)`.
    ///
    /// If the terminal narrowed since an archived row was captured and a 2-column wide character
    /// now sits at `cols - 1` (clipping its continuation cell outside the viewport), returns
    /// `None` so callers never paint or copy a half-clipped wide glyph past the right edge.
    #[must_use]
    pub fn cell(&self, row: u16, col: u16) -> Option<&'a vt100::Cell> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let row_usize = usize::from(row);
        if row_usize < self.history_offset {
            let h_idx = self.history_top + row_usize;
            let cell = self.history?.row(h_idx)?.get(usize::from(col))?;
            if col + 1 == self.cols && cell.is_wide() {
                return None;
            }
            if col == 0 && cell.is_wide_continuation() {
                return None;
            }
            Some(cell)
        } else {
            let live_row = (row_usize - self.history_offset) as u16;
            self.screen.cell(live_row, col)
        }
    }

    /// Whether visible `row` soft-wraps into `row + 1`.
    ///
    /// Always `false` at the seam between the last archived [`History`] row and the first live
    /// parser row so pre-resync and post-resync text never merge into a single logical line.
    #[must_use]
    pub fn row_wrapped(&self, row: u16) -> bool {
        if row >= self.rows {
            return false;
        }
        let row_usize = usize::from(row);
        if row_usize < self.history_offset {
            row_usize + 1 < self.history_offset
                && self
                    .history
                    .is_some_and(|h| h.row_wrapped(self.history_top + row_usize))
        } else {
            let live_row = (row_usize - self.history_offset) as u16;
            self.screen.row_wrapped(live_row)
        }
    }

    /// Plain-text contents of all visible viewport rows, matching [`vt100::Screen::contents`].
    #[must_use]
    pub fn contents(&self) -> String {
        if self.history_offset == 0 {
            return self.screen.contents();
        }
        let mut contents = String::new();
        let mut wrapping = false;
        for row in 0..self.rows {
            self.write_row_slice(&mut contents, row, 0, self.cols, wrapping);
            let wrapped = self.row_wrapped(row);
            if !wrapped {
                contents.push('\n');
            }
            wrapping = wrapped;
        }
        while contents.ends_with('\n') {
            contents.pop();
        }
        contents
    }

    /// Plain-text contents logically between `(start_row, start_col)` and `(end_row, end_col)`,
    /// matching [`vt100::Screen::contents_between`] across archived [`History`] and live rows.
    #[must_use]
    pub fn contents_between(
        &self,
        start_row: u16,
        start_col: u16,
        end_row: u16,
        end_col: u16,
    ) -> String {
        if self.history_offset == 0 {
            return self
                .screen
                .contents_between(start_row, start_col, end_row, end_col);
        }
        if self.rows == 0 || self.cols == 0 || start_row >= self.rows {
            return String::new();
        }
        let start_col = start_col.min(self.cols);
        let end_col = end_col.min(self.cols);
        match start_row.cmp(&end_row) {
            std::cmp::Ordering::Less => {
                let last_row = end_row.min(self.rows - 1);
                let mut contents = String::new();
                for row in start_row..=last_row {
                    if row == start_row {
                        self.write_row_slice(
                            &mut contents,
                            row,
                            start_col,
                            self.cols - start_col,
                            false,
                        );
                        if !self.row_wrapped(row) {
                            contents.push('\n');
                        }
                    } else if row == end_row {
                        self.write_row_slice(&mut contents, row, 0, end_col, false);
                    } else {
                        self.write_row_slice(&mut contents, row, 0, self.cols, false);
                        if !self.row_wrapped(row) {
                            contents.push('\n');
                        }
                    }
                }
                contents
            }
            std::cmp::Ordering::Equal => {
                if start_col < end_col {
                    let mut contents = String::new();
                    self.write_row_slice(
                        &mut contents,
                        start_row,
                        start_col,
                        end_col - start_col,
                        false,
                    );
                    contents
                } else {
                    String::new()
                }
            }
            std::cmp::Ordering::Greater => String::new(),
        }
    }

    /// Order `(start, end)` in either drag direction and return the selected text.
    #[must_use]
    pub fn select_text(&self, start: (u16, u16), end: (u16, u16)) -> String {
        let (s, e) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.contents_between(s.0, s.1, e.0, e.1)
    }

    fn write_row_slice(
        &self,
        contents: &mut String,
        row: u16,
        start: u16,
        width: u16,
        wrapping: bool,
    ) {
        let mut prev_was_wide = false;
        let mut prev_col = start;
        let end = start.saturating_add(width).min(self.cols);
        for col in start..end {
            if prev_was_wide {
                prev_was_wide = false;
                continue;
            }
            let Some(cell) = self.cell(row, col) else {
                continue;
            };
            prev_was_wide = cell.is_wide();
            if cell.has_contents() {
                for _ in 0..(col - prev_col) {
                    contents.push(' ');
                }
                prev_col += col - prev_col;
                contents.push_str(&cell.contents());
                prev_col += if cell.is_wide() { 2 } else { 1 };
            }
        }
        if prev_col == start && wrapping {
            contents.push('\n');
        }
    }
}
