//! Shared task timing semantics: bounded periods spent in Doing, by pane.
use serde::{Deserialize, Serialize};

pub const MAX_PERIODS: usize = 50;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Timeline {
    pub pane_id: Option<u64>,
    pub created_at_ms: Option<u64>,
    pub column_since_ms: Option<u64>,
    pub doing_since_ms: Option<u64>,
    pub started_at_ms: Option<u64>,
    pub done_at_ms: Option<u64>,
    pub work_periods: Vec<WorkPeriod>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkPeriod {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub pane_id: Option<u64>,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().min(u64::MAX as u128) as u64)
}

impl Timeline {
    pub fn move_to(&mut self, from: usize, to: usize, now: u64) -> bool {
        self.transition(from, to, self.pane_id, now)
    }
    pub fn assign(&mut self, column: usize, pane: Option<u64>, now: u64) -> bool {
        self.transition(column, column, pane, now)
    }
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
    pub fn created(now: u64, column: usize, pane: Option<u64>) -> Self {
        let mut value = Self {
            created_at_ms: (now != 0).then_some(now),
            ..Default::default()
        };
        value.transition(0, column, pane, now);
        value
    }

    /// Returns whether timing or assignment changed. Repeating a move normally
    /// does nothing; a legacy Doing card can establish its first known period.
    pub fn transition(&mut self, from: usize, to: usize, pane: Option<u64>, now: u64) -> bool {
        if from > 3 || to > 3 || pane == Some(0) {
            return false;
        }
        let mut changed = self.pane_id != pane || from != to;
        if from != to {
            self.column_since_ms = Some(now);
            if to == 1 {
                self.doing_since_ms = Some(now);
                self.started_at_ms.get_or_insert(now);
            }
            if to == 3 {
                self.done_at_ms = Some(now);
            } else if from == 3 {
                self.done_at_ms = None;
            }
        }
        let open = self
            .work_periods
            .last()
            .filter(|p| p.end_ms.is_none())
            .cloned();
        if let Some(period) = &open
            && (to != 1 || period.pane_id != pane)
        {
            self.work_periods.last_mut().unwrap().end_ms = Some(now.max(period.start_ms));
            changed = true;
        }
        if to == 1 && (from != 1 || open.as_ref().is_none_or(|p| p.pane_id != pane)) {
            let legacy =
                from == 1 && open.is_none() && self.work_periods.is_empty() && pane == self.pane_id;
            let start = if legacy {
                self.doing_since_ms.or(self.started_at_ms).unwrap_or(now)
            } else {
                now
            };
            self.work_periods.push(WorkPeriod {
                start_ms: start,
                end_ms: None,
                pane_id: pane,
            });
            if self.work_periods.len() > MAX_PERIODS {
                self.work_periods
                    .drain(..self.work_periods.len() - MAX_PERIODS);
            }
            changed = true;
        }
        self.pane_id = pane;
        changed
    }

    pub fn closed_periods(&self, now: u64) -> Vec<WorkPeriod> {
        let end = self.done_at_ms.or(self.column_since_ms).unwrap_or(now);
        if self.work_periods.is_empty() {
            return self
                .started_at_ms
                .map(|start| {
                    vec![WorkPeriod {
                        start_ms: start,
                        end_ms: Some(end.max(start)),
                        pane_id: self.pane_id,
                    }]
                })
                .unwrap_or_default();
        }
        self.work_periods
            .iter()
            .map(|p| WorkPeriod {
                start_ms: p.start_ms,
                end_ms: Some(
                    p.end_ms
                        .unwrap_or(self.done_at_ms.unwrap_or(now))
                        .max(p.start_ms),
                ),
                pane_id: p.pane_id.or(self.pane_id),
            })
            .collect()
    }

    pub fn duration_ms(&self, now: u64) -> u64 {
        self.closed_periods(now).iter().fold(0_u64, |total, p| {
            total.saturating_add(p.end_ms.unwrap_or(p.start_ms).saturating_sub(p.start_ms))
        })
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.pane_id == Some(0)
            || self.work_periods.len() > MAX_PERIODS
            || self.work_periods.iter().enumerate().any(|(i, p)| {
                p.pane_id == Some(0)
                    || p.end_ms.is_some_and(|e| e < p.start_ms)
                    || (p.end_ms.is_none() && i + 1 != self.work_periods.len())
            })
        {
            return Err("Invalid task timeline.".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn doing_periods_exclude_review_and_waiting_and_track_reassignment() {
        let mut t = Timeline::created(100, 0, None);
        assert!(t.transition(0, 1, Some(7), 200));
        assert!(t.transition(1, 1, Some(8), 250));
        assert!(t.transition(1, 2, Some(8), 300));
        assert!(t.transition(2, 1, Some(8), 900));
        assert!(t.transition(1, 3, Some(8), 1000));
        assert_eq!(t.duration_ms(2000), 200);
        assert_eq!(t.started_at_ms, Some(200));
        assert_eq!(t.done_at_ms, Some(1000));
        assert_eq!(
            t.work_periods.iter().map(|p| p.pane_id).collect::<Vec<_>>(),
            vec![Some(7), Some(8), Some(8)]
        );
        assert!(!t.transition(3, 3, Some(8), 1100));
        assert!(t.transition(3, 0, Some(8), 1200));
        assert_eq!(t.done_at_ms, None);
        assert_eq!(t.started_at_ms, Some(200));
    }
    #[test]
    fn legacy_dates_are_not_fabricated_and_periods_are_bounded() {
        let mut t: Timeline = serde_json::from_str("{}").unwrap();
        assert_eq!(t.created_at_ms, None);
        assert_eq!(t.duration_ms(1000), 0);
        t.doing_since_ms = Some(100);
        assert!(t.transition(1, 1, None, 200));
        assert_eq!(t.work_periods[0].start_ms, 100);
        assert_eq!(t.created_at_ms, None);
        for n in 0..60 {
            t.transition(1, 2, None, 300 + n * 20);
            t.transition(2, 1, None, 310 + n * 20);
        }
        assert_eq!(t.work_periods.len(), MAX_PERIODS);
        t.validate().unwrap();
    }
    #[test]
    fn creation_in_doing_assigns_one_period_and_open_duration_uses_now() {
        let t = Timeline::created(200, 1, Some(7));
        assert_eq!(t.work_periods.len(), 1);
        assert_eq!(t.work_periods[0].pane_id, Some(7));
        assert_eq!(t.duration_ms(350), 150);
        let with_future: Timeline = serde_json::from_str(r#"{"future_field":true}"#).unwrap();
        assert!(with_future.is_empty());
    }
    #[test]
    fn backwards_clock_cannot_make_negative_work_and_invalid_inputs_are_inert() {
        let mut t = Timeline::created(200, 1, None);
        let original = t.clone();
        assert!(!t.transition(1, 4, None, 250));
        assert!(!t.transition(1, 1, Some(0), 250));
        assert_eq!(t, original);
        t.transition(1, 3, None, 100);
        assert_eq!(t.duration_ms(100), 0);
        t.validate().unwrap();
    }
}
