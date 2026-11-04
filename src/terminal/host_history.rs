//! Bounded display-only cells retained before a remote screen replacement.
//! They never go back through the live parser or produce terminal replies.
use std::collections::VecDeque;

pub const MAX_ROWS: usize = 1000;
pub const MAX_CELLS: usize = 300_000;
pub const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
pub struct History {
    rows: VecDeque<Vec<vt100::Cell>>,
    cells: usize,
}

impl History {
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn row(&self, index: usize) -> Option<&[vt100::Cell]> {
        self.rows.get(index).map(Vec::as_slice)
    }
    /// Copy the old parser's history and visible cells, preserving styles and
    /// wide-cell continuations. Neither remote escape sequences nor stale DEC
    /// margins can affect this copy. The caller replaces the live parser next.
    pub fn capture(&mut self, parser: &mut vt100::Parser) {
        let old_offset = parser.screen().scrollback();
        let (rows, cols) = parser.screen().size();
        parser.set_scrollback(usize::MAX);
        let history = parser.screen().scrollback();
        let total = history + usize::from(rows);
        // Do not allocate rows that would immediately be evicted anyway.
        let retain = MAX_ROWS.min(MAX_CELLS / usize::from(cols).max(1));
        for index in total.saturating_sub(retain)..total {
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
            self.cells += cells.len();
            self.rows.push_back(cells);
            while self.rows.len() > MAX_ROWS || self.cells > MAX_CELLS {
                if let Some(removed) = self.rows.pop_front() {
                    self.cells -= removed.len();
                }
            }
        }
        parser.set_scrollback(old_offset);
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
    }
    #[test]
    fn invalid_snapshot_dimensions_are_refused() {
        assert!(replacement(0, 80, b"x").is_none());
        assert!(replacement(20, 1001, b"x").is_none());
        assert!(replacement(20, 80, &vec![0; MAX_SNAPSHOT_BYTES + 1]).is_none());
    }
}
