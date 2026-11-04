//! Incremental screen application. A large snapshot must not monopolize a frame.
use super::host_history::{self, History, MAX_SNAPSHOT_BYTES};

pub const FRAME_BYTES: usize = 128 * 1024;
pub const OUTPUT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    Size,
    TooLarge,
}

enum Kind {
    Output,
    Snapshot(Option<Box<vt100::Parser>>),
}

// No Debug: screen bytes can contain private terminal output.
pub struct Pending {
    sequence: u64,
    kind: Kind,
    bytes: Vec<u8>,
    offset: usize,
    archive_previous: bool,
}
impl Pending {
    pub fn output(sequence: u64, bytes: Vec<u8>) -> Result<Self, Invalid> {
        if bytes.len() > OUTPUT_BYTES {
            return Err(Invalid::TooLarge);
        }
        Ok(Self {
            sequence,
            kind: Kind::Output,
            bytes,
            offset: 0,
            archive_previous: true,
        })
    }
    pub fn snapshot(sequence: u64, rows: u16, cols: u16, bytes: Vec<u8>) -> Result<Self, Invalid> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(Invalid::TooLarge);
        }
        let parser = host_history::replacement(rows, cols, &[]).ok_or(Invalid::Size)?;
        Ok(Self {
            sequence,
            kind: Kind::Snapshot(Some(Box::new(parser))),
            bytes,
            offset: 0,
            archive_previous: true,
        })
    }
    pub fn is_snapshot(&self) -> bool {
        matches!(self.kind, Kind::Snapshot(_))
    }
    pub fn without_previous_screen(mut self) -> Self {
        self.archive_previous = false;
        self
    }
    pub fn remaining_bytes(&self) -> usize {
        self.bytes.len() - self.offset
    }
    /// Apply live output through the caller's normal DSR/response path, then
    /// advance the position. Completion is never published before emulation.
    pub fn apply_output(&mut self, budget: usize, apply: impl FnOnce(&[u8])) -> usize {
        if self.is_snapshot() {
            return 0;
        }
        let end = self
            .offset
            .saturating_add(budget.min(FRAME_BYTES))
            .min(self.bytes.len());
        apply(&self.bytes[self.offset..end]);
        let applied = end - self.offset;
        self.offset = end;
        applied
    }
    /// A snapshot never goes through the live DSR/response path. Until fully
    /// decoded, the previous visible screen remains intact and input stays off.
    pub fn advance_snapshot(
        &mut self,
        visible: &mut vt100::Parser,
        history: &mut History,
        budget: usize,
    ) -> bool {
        let Kind::Snapshot(fresh) = &mut self.kind else {
            return false;
        };
        let Some(parser) = fresh.as_mut() else {
            return true;
        };
        let end = self
            .offset
            .saturating_add(budget.min(FRAME_BYTES))
            .min(self.bytes.len());
        parser.process(&self.bytes[self.offset..end]);
        self.offset = end;
        if self.offset != self.bytes.len() {
            return false;
        }
        if self.archive_previous {
            history.capture(visible);
        }
        *visible = *fresh.take().expect("snapshot parser");
        true
    }
    pub fn completed_sequence(&self) -> Option<u64> {
        if self.offset != self.bytes.len() {
            return None;
        }
        match self.kind {
            Kind::Output | Kind::Snapshot(None) => Some(self.sequence),
            Kind::Snapshot(Some(_)) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_and_escapes_survive_frame_boundaries_and_sequence_waits() {
        let bytes = "\x1b[31mQuébec 🙂\x1b[0m".as_bytes().to_vec();
        let mut pending = Pending::output(7, bytes.clone()).unwrap();
        let mut visible = vt100::Parser::new(3, 30, 0);
        assert_eq!(pending.completed_sequence(), None);
        while pending.completed_sequence().is_none() {
            pending.apply_output(1, |bytes| visible.process(bytes));
        }
        let mut reference = vt100::Parser::new(3, 30, 0);
        reference.process(&bytes);
        assert_eq!(pending.completed_sequence(), Some(7));
        assert_eq!(
            visible.screen().contents_formatted(),
            reference.screen().contents_formatted()
        );
    }
    #[test]
    fn snapshot_is_applied_atomically_after_bounded_steps() {
        let mut visible = vt100::Parser::new(3, 30, 0);
        visible.process(b"old\x1b[2;3r\x1b[?6h");
        let old = visible.screen().contents();
        let mut history = History::default();
        let mut pending = Pending::snapshot(12, 3, 30, b"\x1b[Hnew screen".to_vec()).unwrap();
        assert!(pending.is_snapshot());
        assert_eq!(
            pending.apply_output(1, |_| panic!("snapshot cannot trigger live replies")),
            0
        );
        assert!(!pending.advance_snapshot(&mut visible, &mut history, 2));
        assert_eq!(pending.completed_sequence(), None);
        assert_eq!(visible.screen().contents(), old);
        assert_eq!(history.len(), 0);
        while !pending.advance_snapshot(&mut visible, &mut history, 2) {}
        assert_eq!(pending.completed_sequence(), Some(12));
        assert_eq!(visible.screen().contents(), "new screen");
        assert_eq!(history.len(), 3);
        assert!(pending.advance_snapshot(&mut visible, &mut history, 2));
        assert_eq!(history.len(), 3, "history archived only once");
    }
    #[test]
    fn invalid_data_and_zero_budget_do_not_change_visible_state() {
        assert!(matches!(
            Pending::output(1, vec![0; OUTPUT_BYTES + 1]),
            Err(Invalid::TooLarge)
        ));
        assert!(matches!(
            Pending::snapshot(1, 0, 30, vec![]),
            Err(Invalid::Size)
        ));
        let mut pending = Pending::output(1, b"x".to_vec()).unwrap();
        assert_eq!(
            pending.apply_output(0, |bytes| assert!(bytes.is_empty())),
            0
        );
        assert_eq!(pending.completed_sequence(), None);
    }
}
