//! Persisted terminal split trees. This metadata never starts or owns a process.
use crate::pane_grid::{self, LeafKind, PaneLeaf, PaneNode, SplitDir};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalLayout {
    pub tree: Option<PaneNode>,
    pub maximized: Option<u64>,
    next_split: u64,
}

impl Default for TerminalLayout {
    fn default() -> Self {
        Self {
            tree: None,
            maximized: None,
            next_split: 1,
        }
    }
}

pub fn project_key(project: Option<u64>) -> String {
    project.map_or_else(|| "global".into(), |id| id.to_string())
}

impl TerminalLayout {
    pub fn validate(&self, allowed: &HashSet<u64>) -> Result<(), String> {
        fn walk(
            node: &PaneNode,
            depth: usize,
            count: &mut usize,
            names: &mut HashSet<String>,
            allowed: &HashSet<u64>,
            next: u64,
        ) -> bool {
            *count += 1;
            if depth > 32 || *count > 299 {
                return false;
            }
            match node {
                PaneNode::Leaf(leaf) => {
                    let Ok(id) = leaf.id.parse::<u64>() else {
                        return false;
                    };
                    allowed.contains(&id)
                        && id.to_string() == leaf.id
                        && names.insert(leaf.id.clone())
                        && matches!(leaf.kind, LeafKind::Shell | LeafKind::Agent)
                        && leaf.worktree_path.is_none()
                        && leaf.view_path.is_none()
                        && !leaf.live_share_page
                }
                PaneNode::Split {
                    id,
                    children,
                    sizes,
                    ..
                } => {
                    let valid_id = id
                        .strip_prefix("split-")
                        .and_then(|s| s.parse::<u64>().ok())
                        .is_some_and(|id| id > 0 && id < next);
                    valid_id
                        && names.insert(id.clone())
                        && (2..=100).contains(&children.len())
                        && sizes.len() == children.len()
                        && sizes
                            .iter()
                            .all(|s| s.is_finite() && *s > 0.0 && *s <= 100.0)
                        && (sizes.iter().sum::<f64>() - 100.0).abs() < 0.01
                        && children
                            .iter()
                            .all(|child| walk(child, depth + 1, count, names, allowed, next))
                }
            }
        }
        if self.next_split == 0
            || self.next_split > u64::MAX - 1_000
            || self.maximized.is_some_and(|id| !allowed.contains(&id))
            || self.tree.as_ref().is_some_and(|tree| {
                !walk(
                    tree,
                    0,
                    &mut 0,
                    &mut HashSet::new(),
                    allowed,
                    self.next_split,
                )
            })
        {
            return Err("Invalid terminal layout metadata".into());
        }
        Ok(())
    }

    pub fn sync(&mut self, ids: &[u64], room: [f64; 2]) -> bool {
        let before = self.clone();
        let keep: HashSet<String> = ids.iter().map(u64::to_string).collect();
        let removed = pane_grid::tree_leaves(self.tree.as_ref(), |leaf| !keep.contains(&leaf.id))
            .into_iter()
            .map(|leaf| leaf.id.clone())
            .collect();
        self.tree = self
            .tree
            .as_ref()
            .and_then(|tree| pane_grid::without_leaves(tree, &removed));
        let mut leaves: Vec<_> = pane_grid::tree_leaves(self.tree.as_ref(), |_| true)
            .into_iter()
            .cloned()
            .collect();
        let present: HashSet<_> = leaves.iter().map(|leaf| leaf.id.clone()).collect();
        let missing: Vec<_> = ids
            .iter()
            .filter(|id| !present.contains(&id.to_string()))
            .collect();
        if !missing.is_empty() {
            leaves.extend(
                missing
                    .into_iter()
                    .map(|id| PaneLeaf::new(id.to_string(), LeafKind::Shell)),
            );
            self.rebuild(leaves, &pane_grid::grid_shape(ids.len(), room[0], room[1]));
        }
        if self.maximized.is_some_and(|id| !ids.contains(&id)) {
            self.maximized = None;
        }
        *self != before
    }

    fn rebuild(&mut self, leaves: Vec<PaneLeaf>, counts: &[usize]) {
        self.next_split = 1;
        let next = &mut self.next_split;
        self.tree = pane_grid::build_grid_tree(leaves, counts, || {
            let id = format!("split-{next}");
            *next += 1;
            id
        });
        self.maximized = None;
    }

    pub fn tile(&mut self, room: [f64; 2], columns: Option<usize>) {
        let leaves: Vec<_> = pane_grid::tree_leaves(self.tree.as_ref(), |_| true)
            .into_iter()
            .cloned()
            .collect();
        let counts = match columns {
            Some(columns) => {
                pane_grid::row_counts(leaves.len(), leaves.len().div_ceil(columns.clamp(1, 3)))
            }
            None => pane_grid::grid_shape(leaves.len(), room[0], room[1]),
        };
        self.rebuild(leaves, &counts);
    }

    pub fn rebuild_ordered(&mut self, ordered_ids: &[u64], row_counts: &[usize]) {
        let existing: std::collections::HashMap<String, PaneLeaf> =
            pane_grid::tree_leaves(self.tree.as_ref(), |_| true)
                .into_iter()
                .map(|leaf| (leaf.id.clone(), leaf.clone()))
                .collect();
        let leaves: Vec<PaneLeaf> = ordered_ids
            .iter()
            .map(|id| {
                let key = id.to_string();
                existing
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| PaneLeaf::new(key, LeafKind::Shell))
            })
            .collect();
        self.rebuild(leaves, row_counts);
    }

    pub fn split(&mut self, target: u64, new: u64, dir: SplitDir) -> bool {
        self.split_with_order(target, new, dir, false)
    }

    pub fn split_with_order(&mut self, target: u64, new: u64, dir: SplitDir, before: bool) -> bool {
        if self.next_split >= u64::MAX - 1_000 {
            return false;
        }
        let Some(tree) = &self.tree else {
            return false;
        };
        if pane_grid::find_leaf(Some(tree), &target.to_string()).is_none() {
            return false;
        }
        // Keep persisted recursion bounded even after repeated splitting of one pane.
        let mut candidate = pane_grid::split_leaf(
            tree,
            &target.to_string(),
            dir,
            PaneLeaf::new(new.to_string(), LeafKind::Shell),
            format!("split-{}", self.next_split),
        );
        if before {
            fn reverse_at(node: &mut PaneNode, split_id: &str) {
                if let PaneNode::Split {
                    id,
                    children,
                    sizes,
                    ..
                } = node
                {
                    if id == split_id {
                        children.reverse();
                        sizes.reverse();
                    } else {
                        for child in children {
                            reverse_at(child, split_id);
                        }
                    }
                }
            }
            reverse_at(&mut candidate, &format!("split-{}", self.next_split));
        }
        let allowed = pane_grid::tree_leaves(Some(&candidate), |_| true)
            .into_iter()
            .filter_map(|leaf| leaf.id.parse().ok())
            .collect();
        let next = Self {
            tree: Some(candidate),
            maximized: None,
            next_split: self.next_split + 1,
        };
        if next.validate(&allowed).is_err() {
            return false;
        }
        *self = next;
        true
    }

    pub fn resize(
        &mut self,
        split: &str,
        index: usize,
        pixels: f64,
        delta: f64,
        key: Option<bool>,
    ) -> bool {
        fn apply(
            node: &mut PaneNode,
            split: &str,
            index: usize,
            pixels: f64,
            delta: f64,
            key: Option<bool>,
        ) -> bool {
            let PaneNode::Split {
                id,
                dir,
                children,
                sizes,
            } = node
            else {
                return false;
            };
            if id == split {
                let next = if let Some(forward) = key {
                    pane_grid::resize_divider_key(children.len(), sizes, index, forward)
                } else {
                    pane_grid::resize_divider_drag(
                        children.len(),
                        sizes,
                        index,
                        *dir,
                        pixels - 4.0 * (children.len() - 1) as f64,
                        delta,
                    )
                };
                if let Some(next) = next
                    && next != *sizes
                {
                    *sizes = next;
                    return true;
                }
                return false;
            }
            children
                .iter_mut()
                .any(|child| apply(child, split, index, pixels, delta, key))
        }
        self.tree
            .as_mut()
            .is_some_and(|tree| apply(tree, split, index, pixels, delta, key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_before_places_only_the_new_pane_left_of_its_target() {
        let mut layout = TerminalLayout::default();
        layout.sync(&[1, 2], [1200.0, 700.0]);
        let old = pane_grid::grid_layout(layout.tree.as_ref());
        let first = old.tiles.iter().find(|t| t.leaf.id == "1").unwrap().rect;
        assert!(layout.split_with_order(2, 3, SplitDir::Row, true));
        let next = pane_grid::grid_layout(layout.tree.as_ref());
        assert_eq!(
            next.tiles.iter().find(|t| t.leaf.id == "1").unwrap().rect,
            first
        );
        let target = next
            .tiles
            .iter()
            .find(|t| t.leaf.id == "2")
            .unwrap()
            .rect
            .to_px(1200.0, 700.0, 4.0);
        let added = next
            .tiles
            .iter()
            .find(|t| t.leaf.id == "3")
            .unwrap()
            .rect
            .to_px(1200.0, 700.0, 4.0);
        assert!(added.right() < target.left);
        assert_eq!(added.top, target.top);
        layout.validate(&HashSet::from([1, 2, 3])).unwrap();
        let saved = layout.clone();
        assert!(!layout.split_with_order(99, 4, SplitDir::Row, true));
        assert_eq!(layout, saved);
        assert!(!layout.split_with_order(2, 1, SplitDir::Row, true));
        assert_eq!(layout, saved);
    }

    #[test]
    fn split_resize_close_and_restore_preserve_session_identity() {
        let mut layout = TerminalLayout::default();
        assert!(layout.sync(&[1, 2], [1200.0, 700.0]));
        assert!(layout.split(2, 3, SplitDir::Col));
        let geometry = pane_grid::grid_layout(layout.tree.as_ref());
        assert_eq!(geometry.tiles.len(), 3);
        let divider = &geometry.dividers[0];
        assert!(layout.resize(&divider.split_id, divider.index, 1200.0, 70.0, None));
        let before = layout.clone();
        assert!(!layout.sync(&[1, 2, 3], [1200.0, 700.0]));
        assert_eq!(layout, before);
        let json = serde_json::to_string(&layout).unwrap();
        let mut restored: TerminalLayout = serde_json::from_str(&json).unwrap();
        restored.validate(&HashSet::from([1, 2, 3])).unwrap();
        assert_eq!(restored, layout);
        restored.maximized = Some(2);
        assert!(restored.sync(&[1, 3], [1200.0, 700.0]));
        assert_eq!(restored.maximized, None);
        restored.validate(&HashSet::from([1, 3])).unwrap();
        assert_eq!(
            pane_grid::grid_layout(restored.tree.as_ref()).tiles.len(),
            2
        );
        assert!(restored.sync(&[], [800.0, 600.0]));
        assert!(restored.tree.is_none());
    }

    #[test]
    fn imported_layouts_reject_foreign_sessions_duplicate_ids_and_invalid_sizes() {
        let mut layout = TerminalLayout::default();
        layout.sync(&[1, 2], [1200.0, 700.0]);
        assert!(layout.validate(&HashSet::from([1])).is_err());
        let mut bad = layout.clone();
        if let Some(PaneNode::Split { sizes, .. }) = &mut bad.tree {
            sizes[0] = f64::NAN;
        }
        assert!(bad.validate(&HashSet::from([1, 2])).is_err());
        if let Some(PaneNode::Split { children, .. }) = &mut layout.tree {
            children[1] = children[0].clone();
        }
        assert!(layout.validate(&HashSet::from([1, 2])).is_err());
    }

    #[test]
    fn repeated_splits_are_bounded_and_auto_tile_preserves_all_sessions() {
        let mut layout = TerminalLayout::default();
        layout.sync(&[1], [1200.0, 700.0]);
        let mut ids = vec![1];
        for id in 2..100 {
            if !layout.split(1, id, SplitDir::Row) {
                break;
            }
            ids.push(id);
        }
        assert!(ids.len() <= 33);
        layout.validate(&ids.iter().copied().collect()).unwrap();
        layout.tile([1200.0, 700.0], None);
        layout.validate(&ids.iter().copied().collect()).unwrap();
        assert_eq!(
            pane_grid::grid_layout(layout.tree.as_ref()).tiles.len(),
            ids.len()
        );
    }
}
