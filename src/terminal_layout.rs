//! Persisted terminal split trees. This metadata never starts or owns a process.
use crate::pane_grid::{self, LeafKind, PaneLeaf, PaneNode, SplitDir};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum GridMode {
    Manual,
    Tiled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminalLayout {
    pub tree: Option<PaneNode>,
    #[serde(skip_serializing, deserialize_with = "read_legacy_maximized")]
    pub maximized: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grid_mode: Option<GridMode>,
    next_split: u64,
}

// Older workspace files stored this transient view setting alongside the split tree.
fn read_legacy_maximized<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    Option::<u64>::deserialize(deserializer)?;
    Ok(None)
}

impl Default for TerminalLayout {
    fn default() -> Self {
        Self {
            tree: None,
            maximized: None,
            focused: None,
            grid_mode: None,
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
                        && leaf
                            .worktree_path
                            .as_ref()
                            .is_none_or(|p| !p.is_empty() && p.len() <= 4096 && !p.contains('\0'))
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
            || self.focused.is_some_and(|id| !allowed.contains(&id))
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
        self.sync_with_tiling(ids, room, true)
    }

    pub fn sync_with_tiling(&mut self, ids: &[u64], room: [f64; 2], auto_tile: bool) -> bool {
        let focused = self.focused.filter(|id| ids.contains(id));
        let maximized = self.maximized.filter(|id| ids.contains(id));
        let selection_changed = self.focused != focused || self.maximized != maximized;
        self.focused = focused;
        self.maximized = maximized;
        let was_tiled = match self.grid_mode {
            Some(GridMode::Manual) => false,
            Some(GridMode::Tiled) => true,
            None => pane_grid::is_near_grid(self.tree.as_ref()),
        };
        let keep: HashSet<_> = ids.iter().map(u64::to_string).collect();
        let removed: HashSet<_> =
            pane_grid::tree_leaves(self.tree.as_ref(), |leaf| !keep.contains(&leaf.id))
                .into_iter()
                .map(|leaf| leaf.id.clone())
                .collect();
        if !removed.is_empty() {
            self.tree = self
                .tree
                .as_ref()
                .and_then(|tree| pane_grid::without_leaves(tree, &removed));
        }
        let leaves = pane_grid::tree_leaves(self.tree.as_ref(), |_| true);
        let present: HashSet<_> = leaves.iter().map(|leaf| leaf.id.as_str()).collect();
        let missing: Vec<_> = ids
            .iter()
            .copied()
            .filter(|id| !present.contains(id.to_string().as_str()))
            .collect();
        let membership_changed = !removed.is_empty() || !missing.is_empty();
        if !missing.is_empty() {
            if leaves.is_empty() || (auto_tile && (leaves.len() == 1 || was_tiled)) {
                let mut ordered: Vec<_> = leaves.into_iter().cloned().collect();
                ordered.extend(
                    missing
                        .into_iter()
                        .map(|id| PaneLeaf::new(id.to_string(), LeafKind::Shell)),
                );
                self.rebuild(ordered, &pane_grid::grid_shape(ids.len(), room[0], room[1]));
                self.grid_mode = Some(GridMode::Tiled);
            } else {
                for id in missing {
                    if !self.split_largest(id, room) {
                        // At the depth limit a flat grid can still hold every live session.
                        let ordered = ids
                            .iter()
                            .map(|id| PaneLeaf::new(id.to_string(), LeafKind::Shell))
                            .collect();
                        self.rebuild(ordered, &pane_grid::grid_shape(ids.len(), room[0], room[1]));
                        self.grid_mode = Some(GridMode::Tiled);
                        break;
                    }
                }
            }
        } else if !removed.is_empty() && auto_tile && was_tiled {
            self.tile(room, None);
        }
        membership_changed || selection_changed
    }

    fn split_largest(&mut self, id: u64, room: [f64; 2]) -> bool {
        let mut best = None;
        for tile in pane_grid::grid_layout(self.tree.as_ref()).tiles {
            let rect = tile.rect.to_px(room[0], room[1], 0.);
            let area = rect.width * rect.height;
            if best.is_none_or(|(largest, _, _)| area > largest) {
                let dir = if rect.width >= rect.height {
                    SplitDir::Row
                } else {
                    SplitDir::Col
                };
                best = tile.leaf.id.parse().ok().map(|target| (area, target, dir));
            }
        }
        best.is_some_and(|(_, target, dir)| self.split(target, id, dir))
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
        self.grid_mode = Some(GridMode::Tiled);
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
        self.grid_mode = None;
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
            focused: self.focused,
            grid_mode: Some(GridMode::Manual),
            next_split: self.next_split + 1,
        };
        if next.validate(&allowed).is_err() {
            return false;
        }
        *self = next;
        true
    }

    pub fn move_pane(&mut self, from: u64, to: Option<u64>, zone: pane_grid::DropZone) -> bool {
        let Some(tree) = self.tree.as_ref() else {
            return false;
        };
        let Some(candidate) = pane_grid::reposition_leaf(
            tree,
            &from.to_string(),
            to.map(|id| id.to_string()).as_deref(),
            zone,
            format!("split-{}", self.next_split),
        ) else {
            return false;
        };
        let allowed = pane_grid::tree_leaves(Some(tree), |_| true)
            .into_iter()
            .filter_map(|leaf| leaf.id.parse().ok())
            .collect();
        let next = Self {
            tree: Some(candidate),
            maximized: None,
            focused: Some(from),
            grid_mode: if zone == pane_grid::DropZone::Center {
                self.grid_mode
            } else {
                Some(GridMode::Manual)
            },
            next_split: self
                .next_split
                .saturating_add(u64::from(zone != pane_grid::DropZone::Center)),
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
        let changed = self
            .tree
            .as_mut()
            .is_some_and(|tree| apply(tree, split, index, pixels, delta, key));
        if changed {
            self.grid_mode = Some(GridMode::Manual);
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_drop_persists_layout_and_preserves_auto_tiling_for_a_swap() {
        use pane_grid::DropZone;
        let mut layout = TerminalLayout::default();
        let room = [1000., 600.];
        layout.sync(&[1, 2, 3], room);
        let mode = layout.grid_mode;
        assert!(layout.move_pane(1, Some(2), DropZone::Center));
        assert_eq!(layout.grid_mode, mode);
        assert!(layout.move_pane(1, Some(3), DropZone::Top));
        assert_eq!(layout.grid_mode, Some(GridMode::Manual));
        assert!(layout.move_pane(2, None, DropZone::Right));
        let Some(PaneNode::Split {
            dir,
            children,
            sizes,
            ..
        }) = &layout.tree
        else {
            panic!()
        };
        assert_eq!(*dir, SplitDir::Row);
        assert_eq!(sizes, &[65., 35.]);
        assert!(matches!(&children[1], PaneNode::Leaf(leaf) if leaf.id == "2"));
        assert!(matches!(&children[0], PaneNode::Split { .. }));
        let mut restored: TerminalLayout =
            serde_json::from_slice(&serde_json::to_vec(&layout).unwrap()).unwrap();
        assert!(!restored.sync(&[1, 2, 3], room));
        assert_eq!(restored, layout);
        assert!(!restored.move_pane(1, Some(7), DropZone::Left));
        assert_eq!(restored, layout);
    }

    #[test]
    fn manual_grid_adds_to_the_largest_pane_and_arrange_resumes_tiling() {
        let room = [1200., 600.];
        let mut layout = TerminalLayout::default();
        layout.sync(&[1], room);
        assert!(layout.split(1, 2, SplitDir::Row));
        let divider = pane_grid::grid_layout(layout.tree.as_ref())
            .dividers
            .remove(0);
        assert!(layout.resize(&divider.split_id, 0, 1200., 160., None));
        let bounds = |layout: &TerminalLayout, id: &str| {
            pane_grid::grid_layout(layout.tree.as_ref())
                .tiles
                .into_iter()
                .find(|tile| tile.leaf.id == id)
                .unwrap()
                .rect
                .to_px(room[0], room[1], 0.)
        };
        let small = bounds(&layout, "2");
        let saved = serde_json::to_value(&layout).unwrap();
        for legacy in [false, true] {
            let mut json = saved.clone();
            if legacy {
                json.as_object_mut().unwrap().remove("grid_mode");
            }
            let mut restored: TerminalLayout = serde_json::from_value(json).unwrap();
            restored.sync(&[1, 2, 3], room);
            assert_eq!(bounds(&restored, "2"), small);
            assert_eq!(bounds(&restored, "1").width, bounds(&restored, "3").width);
            assert_eq!(bounds(&restored, "1").top, bounds(&restored, "3").top);
            restored.sync(&[1, 2], room);
            assert_eq!(bounds(&restored, "2"), small);
            restored.tile(room, None);
            restored.sync(&[1, 2, 3, 4], room);
            assert!(pane_grid::is_near_grid(restored.tree.as_ref()));
            restored.sync(&[1, 3, 4], room);
            assert!(pane_grid::is_near_grid(restored.tree.as_ref()));
        }
    }

    #[test]
    fn equal_explicit_split_stays_manual_after_reload_until_only_one_pane_remains() {
        let room = [1000., 600.];
        let mut layout = TerminalLayout::default();
        layout.sync(&[1], room);
        layout.split(1, 2, SplitDir::Row);
        let mut layout: TerminalLayout =
            serde_json::from_slice(&serde_json::to_vec(&layout).unwrap()).unwrap();
        layout.sync(&[1, 2, 3], room);
        let grid = pane_grid::grid_layout(layout.tree.as_ref());
        assert_eq!(
            grid.tiles
                .iter()
                .map(|t| t.leaf.id.as_str())
                .collect::<Vec<_>>(),
            ["1", "3", "2"]
        );
        assert_eq!(grid.tiles[0].rect.to_px(1000., 600., 0.).height, 300.);
        layout.sync(&[2], room);
        layout.sync(&[2, 4], room);
        assert_eq!(layout.grid_mode, Some(GridMode::Tiled));
    }

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
    fn maximization_is_transient_for_current_and_legacy_layouts() {
        let mut layout = TerminalLayout::default();
        layout.sync(&[1, 2], [800., 600.]);
        layout.focused = Some(2);
        layout.maximized = Some(2);
        let mut saved = serde_json::to_value(&layout).unwrap();
        assert!(saved.get("maximized").is_none());
        saved["maximized"] = 2.into();
        let restored: TerminalLayout = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.maximized, None);
        assert_eq!(restored.focused, Some(2));
        assert_eq!(restored.tree, layout.tree);
    }

    #[test]
    fn legacy_layouts_load_and_focused_panes_must_belong_to_the_project() {
        let mut layout: TerminalLayout =
            serde_json::from_str(r#"{"tree":null,"maximized":null,"next_split":1}"#).unwrap();
        assert_eq!(layout.focused, None);
        layout.sync(&[3, 4], [800., 600.]);
        layout.focused = Some(4);
        assert!(layout.split(3, 5, SplitDir::Row));
        assert_eq!(layout.focused, Some(4));
        assert!(layout.validate(&HashSet::from([3, 4, 5])).is_ok());
        layout.focused = Some(99);
        assert!(layout.validate(&HashSet::from([3, 4, 5])).is_err());
        layout.sync(&[3, 4, 5], [800., 600.]);
        assert_eq!(layout.focused, None);
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
