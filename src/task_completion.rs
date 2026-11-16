//! One live owner-task turn. Never serialized or reconstructed from history.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Outcome {
    Pending,
    Complete,
    #[default]
    Cancelled,
}

#[derive(Default)]
pub struct Watch {
    token: u64,
    outcome: Outcome,
    marked: bool,
}

impl Watch {
    pub fn arm(&mut self) {
        self.token = crate::chat::next_input_id();
        self.outcome = Outcome::Pending;
        self.marked = false;
    }
    pub fn token(&self) -> u64 {
        self.token
    }
    pub fn outcome(&self, token: u64) -> Outcome {
        if token == 0 || token != self.token {
            Outcome::Cancelled
        } else {
            self.outcome
        }
    }
    pub fn message(&mut self, text: &str) {
        if self.outcome == Outcome::Pending {
            // Only a complete assistant message is passed here, never a stream delta.
            self.marked = text.len() <= 128 * 1024 && contains_marker(text);
        }
    }
    pub fn finish(&mut self, success: bool) {
        if self.outcome == Outcome::Pending {
            self.outcome = if success && self.marked {
                Outcome::Complete
            } else {
                Outcome::Cancelled
            };
        }
    }
    pub fn cancel(&mut self) {
        self.outcome = Outcome::Cancelled;
        self.marked = false;
    }
}

// Matches Vue agentLimit.js detectTaskDone, including JavaScript whitespace.
fn space(c: char) -> bool {
    matches!(c, '\u{0009}'..='\u{000d}' | ' ' | '\u{00a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}
fn contains_marker(text: &str) -> bool {
    text.split('\n').any(|line| {
        let line = line.trim_matches(space);
        let line = line
            .strip_prefix(['●', '⏺', '•', '*', '>', '-'])
            .unwrap_or(line)
            .trim_start_matches(space);
        matches!(line, "TASK_COMPLETE" | "TASK_COMPLETE.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_completion_matches_all_extracted_vue_vectors() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/task_completion.json")).unwrap();
        let cases = fixture["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 38);
        for case in cases {
            let text = case["text"].as_str().unwrap();
            assert_eq!(
                contains_marker(text),
                case["expected"].as_bool().unwrap(),
                "{text:?}"
            );
        }
    }
    #[test]
    fn task_completion_marker_matches_vue_contract_without_instruction_echoes() {
        for text in [
            "All tests pass.\nTASK_COMPLETE",
            "● TASK_COMPLETE.",
            "\r\n⏺ TASK_COMPLETE",
            "  TASK_COMPLETE.  ",
            "\u{feff}>TASK_COMPLETE\u{00a0}",
        ] {
            assert!(contains_marker(text), "{text:?}");
        }
        for text in [
            "MY_TASK_COMPLETED_FLAG",
            "I will print TASK_COMPLETE when finished.",
            "Next: tests, then TASK_COMPLETE",
            "**TASK_COMPLETE**",
            "TASK_COMPLETE..",
            "\u{85}TASK_COMPLETE",
            "end with TASK and COMPLETE joined by an underscore",
        ] {
            assert!(!contains_marker(text), "{text:?}");
        }
    }
    #[test]
    fn task_completion_requires_success_latest_message_and_live_token() {
        let mut watch = Watch::default();
        watch.message("TASK_COMPLETE");
        watch.finish(true);
        assert!(watch.outcome(0) == Outcome::Cancelled);
        watch.arm();
        let old = watch.token();
        watch.message("TASK_COMPLETE");
        assert!(watch.outcome(old) == Outcome::Pending);
        watch.message("Still working");
        watch.finish(true);
        assert!(watch.outcome(old) == Outcome::Cancelled);
        watch.arm();
        let current = watch.token();
        watch.message("TASK_COMPLETE");
        watch.finish(false);
        assert!(watch.outcome(current) == Outcome::Cancelled);
        watch.arm();
        let current = watch.token();
        watch.message("TASK_COMPLETE");
        watch.finish(true);
        assert!(watch.outcome(current) == Outcome::Complete);
        assert!(watch.outcome(old) == Outcome::Cancelled);
        watch.cancel();
        watch.finish(true);
        assert!(watch.outcome(current) == Outcome::Cancelled);
    }
}
