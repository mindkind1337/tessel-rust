//! Bounded display-only cells retained before a remote screen replacement.
//! They never go back through the live parser or produce terminal replies.
use std::collections::VecDeque;

pub const MAX_ROWS: usize = 1000;
pub const MAX_CELLS: usize = 300_000;
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

struct ArchivedRow {
    cells: Vec<vt100::Cell>,
    wrapped: bool,
}

#[derive(Clone, Copy)]
pub struct CaptureOrigin {
    pub skipped_front: usize,
    pub visible_top: usize,
    pub scrollback: usize,
}

struct Capture {
    first_id: u64,
    origin: CaptureOrigin,
}

#[derive(Default)]
pub struct History {
    rows: VecDeque<ArchivedRow>,
    cells: usize,
    evicted: u64,
    captures: VecDeque<Capture>,
}

impl History {
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn row(&self, index: usize) -> Option<&[vt100::Cell]> {
        self.rows.get(index).map(|row| row.cells.as_slice())
    }
    pub fn row_wrapped(&self, index: usize) -> bool {
        self.rows.get(index).is_some_and(|row| row.wrapped)
    }
    /// Monotonic count of archived rows evicted from the front of this buffer.
    pub fn evicted_rows(&self) -> u64 {
        self.evicted
    }
    /// Monotonic count of all rows ever appended to this buffer (`evicted_rows + len`).
    pub fn total_captured_rows(&self) -> u64 {
        self.evicted.saturating_add(self.rows.len() as u64)
    }
    /// Metadata for the first capture after a polling checkpoint. Records are
    /// discarded when all their rows are evicted; at most MAX_ROWS are retained.
    pub fn capture_origin(&self, first_id: u64) -> Option<CaptureOrigin> {
        self.captures
            .iter()
            .find(|capture| capture.first_id == first_id)
            .map(|capture| capture.origin)
    }
    /// Copy the old parser's history and visible cells, preserving styles and
    /// wide-cell continuations. Neither remote escape sequences nor stale DEC
    /// margins can affect this copy. The caller replaces the live parser next.
    /// An alternate screen is copied after the primary history; this consumes
    /// the old parser's active-grid state. Primary-only captures restore offset.
    pub fn capture(&mut self, parser: &mut vt100::Parser) {
        let old_offset = parser.screen().scrollback();
        let (rows, cols) = parser.screen().size();
        let retain = MAX_ROWS.min(MAX_CELLS / usize::from(cols).max(1));
        let alternate = parser.screen().alternate_screen();
        let mut alternate_rows = Vec::new();
        if alternate {
            for row in usize::from(rows).saturating_sub(retain)..usize::from(rows) {
                let cells = (0..cols)
                    .filter_map(|col| parser.screen().cell(row as u16, col).cloned())
                    .collect();
                let wrapped =
                    row + 1 < usize::from(rows) && parser.screen().row_wrapped(row as u16);
                alternate_rows.push(ArchivedRow { cells, wrapped });
            }
            // The parser is about to be replaced. Do not inject an escape into
            // a possibly unfinished remote OSC/DCS sequence to switch grids.
            parser.use_primary_screen_for_archive();
        }
        parser.set_scrollback(usize::MAX);
        let history = parser.screen().scrollback();
        let primary_total = history + usize::from(rows);
        let total = primary_total + if alternate { usize::from(rows) } else { 0 };
        // Do not allocate rows that would immediately be evicted anyway.
        if total > 0 {
            self.captures.push_back(Capture {
                first_id: self.total_captured_rows(),
                origin: CaptureOrigin {
                    skipped_front: total.saturating_sub(retain),
                    visible_top: if alternate {
                        primary_total
                    } else {
                        history.saturating_sub(old_offset)
                    },
                    scrollback: old_offset,
                },
            });
        }
        for index in total.saturating_sub(retain)..primary_total {
            let row = if index < history {
                parser.set_scrollback(history - index);
                0
            } else {
                parser.set_scrollback(0);
                (index - history) as u16
            };
            let cells = (0..cols)
                .filter_map(|col| parser.screen().cell(row, col).cloned())
                .collect::<Vec<_>>();
            // Never carry soft-wrap across the boundary into a later capture or live screen.
            let wrapped = index + 1 < primary_total && parser.screen().row_wrapped(row);
            self.push_row(ArchivedRow { cells, wrapped });
        }
        for row in alternate_rows {
            self.push_row(row);
        }
        while self
            .captures
            .get(1)
            .is_some_and(|capture| capture.first_id <= self.evicted)
        {
            self.captures.pop_front();
        }
        parser.set_scrollback(old_offset);
    }

    fn push_row(&mut self, row: ArchivedRow) {
        self.cells += row.cells.len();
        self.rows.push_back(row);
        while self.rows.len() > MAX_ROWS || self.cells > MAX_CELLS {
            if let Some(removed) = self.rows.pop_front() {
                self.cells -= removed.cells.len();
                self.evicted = self.evicted.saturating_add(1);
            }
        }
    }
}

/// Replace all emulator modes as well as cells. Processing a snapshot into the
/// old parser can preserve stale margins/origin/alternate-screen state.
pub fn replacement(rows: u16, cols: u16, snapshot: &[u8]) -> Option<vt100::Parser> {
    if !(1..=500).contains(&rows)
        || !(1..=1000).contains(&cols)
        || snapshot.len() > MAX_SNAPSHOT_BYTES
    {
        return None;
    }
    let mut parser = vt100::Parser::new(rows, cols, MAX_ROWS);
    parser.process(snapshot);
    Some(parser)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_keeps_old_scrollback_and_styled_wide_cells() {
        let mut parser = vt100::Parser::new(3, 10, 10);
        parser.process("old\r\nsecond\r\n\x1b[31;4m🙂é\x1b[0m\r\nlast".as_bytes());
        parser.set_scrollback(1);
        let mut history = History::default();
        assert!(history.is_empty());
        history.capture(&mut parser);
        assert!(!history.is_empty());
        let origin = history.capture_origin(0).unwrap();
        assert_eq!(origin.skipped_front, 0);
        assert_eq!(origin.visible_top, 0);
        assert_eq!(parser.screen().scrollback(), 1);
        assert_eq!(history.len(), 4);
        assert_eq!(history.row(0).unwrap()[0].contents(), "o");
        let wide = &history.row(2).unwrap()[0];
        assert_eq!(wide.contents(), "🙂");
        assert!(wide.is_wide());
        assert!(wide.underline());
        assert_eq!(wide.fgcolor(), vt100::Color::Idx(1));
        assert!(history.row(2).unwrap()[1].is_wide_continuation());
    }
    #[test]
    fn fresh_snapshot_does_not_inherit_dec_modes_or_scroll_region() {
        let mut old = vt100::Parser::new(5, 20, 10);
        old.process(b"old\x1b[2;4r\x1b[?6h\x1b[?1049h\x1b[?1h\x1b[?2004h");
        let mut history = History::default();
        history.capture(&mut old);
        let mut fresh = replacement(5, 20, b"\x1b[Hnew").unwrap();
        assert!(!fresh.screen().alternate_screen());
        assert!(!fresh.screen().application_cursor());
        assert!(!fresh.screen().bracketed_paste());
        assert_eq!(fresh.screen().cell(0, 0).unwrap().contents(), "n");
        fresh.process(b"\x1b[5;1Hbottom");
        assert_eq!(fresh.screen().cell(4, 0).unwrap().contents(), "b");
    }
    #[test]
    fn repeated_resync_history_is_bounded_in_rows_and_cells() {
        let mut parser = vt100::Parser::new(500, 1000, 0);
        parser.process(b"sample");
        let mut history = History::default();
        history.capture(&mut parser);
        history.capture(&mut parser);
        assert_eq!(history.len(), 300);
        assert_eq!(history.cells, MAX_CELLS);
        let mut narrow = vt100::Parser::new(500, 1, 0);
        for _ in 0..4 {
            history.capture(&mut narrow);
        }
        assert_eq!(history.len(), MAX_ROWS);
        assert!(history.cells <= MAX_CELLS);
        assert!(history.captures.len() <= history.len());
    }
    #[test]
    fn invalid_snapshot_dimensions_are_refused() {
        assert!(replacement(0, 80, b"x").is_none());
        assert!(replacement(20, 1001, b"x").is_none());
        assert!(replacement(20, 80, &vec![0; MAX_SNAPSHOT_BYTES + 1]).is_none());
    }
    #[test]
    fn alternate_capture_preserves_primary_history_even_inside_an_unfinished_control_string() {
        let mut parser = vt100::Parser::new(2, 8, 10);
        parser.process(b"P0\r\nP1\r\nP2\x1b[?1049hALT0\r\nALT1\x1b]0;unfinished");
        assert!(parser.screen().alternate_screen());
        let mut history = History::default();
        history.capture(&mut parser);
        assert_eq!(history.len(), 5);
        for (row, expected) in ["P0", "P1", "P2", "ALT0", "ALT1"].iter().enumerate() {
            let text: String = history
                .row(row)
                .unwrap()
                .iter()
                .map(vt100::Cell::contents)
                .collect();
            assert_eq!(text.trim_end(), *expected);
        }
        assert!(!history.row_wrapped(2));
        assert!(!history.row_wrapped(4));
        let origin = history.capture_origin(0).unwrap();
        assert_eq!(origin.visible_top, 3);
        assert_eq!(origin.scrollback, 0);

        let mut large = vt100::Parser::new(500, 1000, 1000);
        large.process(b"primary\x1b[?1049h\x1b[500;1HALTEND");
        let mut bounded = History::default();
        bounded.capture(&mut large);
        assert_eq!(bounded.len(), 300);
        assert_eq!(bounded.cells, MAX_CELLS);
        let last: String = bounded
            .row(299)
            .unwrap()
            .iter()
            .map(vt100::Cell::contents)
            .collect();
        assert_eq!(last.trim_end(), "ALTEND");
    }

    #[test]
    fn capture_preserves_internal_soft_wrap_breaks_capture_seam_and_tracks_evictions() {
        let mut parser = vt100::Parser::new(2, 5, 10);
        parser.process(b"1234567890");
        let mut history = History::default();
        assert_eq!(history.evicted_rows(), 0);
        assert_eq!(history.total_captured_rows(), 0);
        history.capture(&mut parser);
        assert_eq!(history.len(), 2);
        assert!(history.row_wrapped(0));
        assert!(
            !history.row_wrapped(1),
            "capture boundary never soft-wraps into a later screen"
        );
        assert_eq!(history.evicted_rows(), 0);
        assert_eq!(history.total_captured_rows(), 2);

        let mut wide_batch = vt100::Parser::new(500, 1000, 0);
        history.capture(&mut wide_batch);
        assert_eq!(history.len(), 300);
        assert_eq!(history.evicted_rows(), 2);
        assert_eq!(history.total_captured_rows(), 302);
    }
}
