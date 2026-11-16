//! Legacy #n addresses are stable within a project, not internal pane ids.
use std::collections::{BTreeMap, BTreeSet};

pub fn numbers(
    entries: impl IntoIterator<Item = (u64, Option<u64>, Option<u32>)>,
) -> BTreeMap<u64, u32> {
    let mut used: BTreeMap<Option<u64>, BTreeSet<u32>> = BTreeMap::new();
    let mut result = BTreeMap::new();
    let mut pending = Vec::new();
    for (pane, project, number) in entries {
        if let Some(number) = number.filter(|number| *number > 0)
            && used.entry(project).or_default().insert(number)
        {
            result.insert(pane, number);
        } else {
            pending.push((pane, project));
        }
    }
    for (pane, project) in pending {
        let taken = used.entry(project).or_default();
        // The application's session count is bounded far below u32::MAX.
        let number = (1..=u32::MAX)
            .find(|number| !taken.contains(number))
            .expect("available pane number");
        taken.insert(number);
        result.insert(pane, number);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_preserve_existing_aliases_and_do_not_use_internal_ids() {
        let value = numbers([
            (900, Some(1), Some(4)),
            (7, Some(1), None),
            (88, Some(2), Some(4)),
        ]);
        assert_eq!(value[&900], 4);
        assert_eq!(value[&7], 1);
        assert_eq!(value[&88], 4);
        let reordered = numbers([
            (88, Some(2), Some(4)),
            (7, Some(1), Some(1)),
            (900, Some(1), Some(4)),
        ]);
        assert_eq!(value, reordered);
    }
    #[test]
    fn repair_duplicates_and_zero_without_stealing_a_later_valid_number() {
        let value = numbers([
            (20, Some(1), Some(2)),
            (30, Some(1), Some(2)),
            (40, Some(1), Some(0)),
            (50, Some(1), Some(1)),
        ]);
        assert_eq!(value, BTreeMap::from([(20, 2), (30, 3), (40, 4), (50, 1)]));
        let after_close = numbers([
            (20, Some(1), Some(2)),
            (40, Some(1), Some(4)),
            (50, Some(1), Some(1)),
            (60, Some(1), None),
        ]);
        assert_eq!(after_close[&60], 3);
    }
}
