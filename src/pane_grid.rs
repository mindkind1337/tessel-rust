//! Center pane split-tree, auto-tiling, divider geometry, geometric `Alt+Arrows` focus,
//! per-worktree view layers, `ViewTabs` routing, and `gridReturnLeaf` invariants.
//!
//! Transcribed directly from:
//! - `src/renderer/src/paneGrid.js`
//! - `src/renderer/src/paneViews.js`
//! - `src/renderer/src/components/PaneGrid.vue`
//! - `src/renderer/src/App.vue` (`showViewTab`, `gridReturnLeaf`, `closeLeaf`, `moveFocus`)

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub const TARGET_ASPECT: f64 = 1.2;
pub const MIN_CELL_WIDTH: f64 = 360.0;
pub const MIN_CELL_HEIGHT: f64 = 160.0;

pub const MIN_PANE_WIDTH: f64 = 325.0;
pub const MIN_PANE_HEIGHT: f64 = 156.0;

pub const DIVIDER_KEY_STEP_PCT: f64 = 2.0;
pub const DIVIDER_MIN_PCT: f64 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitDir {
    Row,
    Col,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeafKind {
    Shell,
    Agent,
    Editor,
    Browser,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneLeaf {
    pub id: String,
    pub kind: LeafKind,
    pub worktree_path: Option<String>,
    pub view_path: Option<String>,
    pub live_share_page: bool,
}

impl PaneLeaf {
    pub fn new(id: impl Into<String>, kind: LeafKind) -> Self {
        Self {
            id: id.into(),
            kind,
            worktree_path: None,
            view_path: None,
            live_share_page: false,
        }
    }

    pub fn with_worktree(mut self, path: impl Into<String>) -> Self {
        self.worktree_path = Some(path.into());
        self
    }

    pub fn with_view_path(mut self, path: impl Into<String>) -> Self {
        self.view_path = Some(path.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PaneNode {
    Leaf(PaneLeaf),
    Split {
        id: String,
        dir: SplitDir,
        children: Vec<PaneNode>,
        sizes: Vec<f64>,
    },
}

impl PaneNode {
    pub fn split(
        id: impl Into<String>,
        dir: SplitDir,
        children: Vec<PaneNode>,
        sizes: Vec<f64>,
    ) -> Self {
        Self::Split {
            id: id.into(),
            dir,
            children,
            sizes,
        }
    }

    pub fn equal_split(id: impl Into<String>, dir: SplitDir, children: Vec<PaneNode>) -> Self {
        let n = children.len();
        let share = if n > 0 { 100.0 / n as f64 } else { 0.0 };
        Self::split(id, dir, children, vec![share; n])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridPreset {
    OneByTwo,
    TwoByOne,
    TwoByTwo,
    ThreeByTwo,
    ThreeByThree,
}

impl GridPreset {
    pub const ALL: [Self; 5] = [
        Self::OneByTwo,
        Self::TwoByOne,
        Self::TwoByTwo,
        Self::ThreeByTwo,
        Self::ThreeByThree,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::OneByTwo => "1x2",
            Self::TwoByOne => "2x1",
            Self::TwoByTwo => "2x2",
            Self::ThreeByTwo => "3x2",
            Self::ThreeByThree => "3x3",
        }
    }

    /// `(cols, rows)` -> row counts for an even preset grid.
    pub fn row_counts(self) -> Vec<usize> {
        let (cols, rows) = match self {
            Self::OneByTwo => (1, 2),
            Self::TwoByOne => (2, 1),
            Self::TwoByTwo => (2, 2),
            Self::ThreeByTwo => (3, 2),
            Self::ThreeByThree => (3, 3),
        };
        vec![cols; rows]
    }
}

/// `n` panes distributed across `rows` rows, longer rows first (`rowCounts(5, 2) == [3, 2]`).
pub fn row_counts(n: usize, rows: usize) -> Vec<usize> {
    if rows == 0 {
        return Vec::new();
    }
    let base = n / rows;
    let extra = n % rows;
    (0..rows).map(|i| base + usize::from(i < extra)).collect()
}

fn shape_cost(counts: &[usize], width: f64, height: f64) -> f64 {
    let rows = counts.len();
    let n: usize = counts.iter().sum();
    if rows == 0 || n == 0 {
        return f64::INFINITY;
    }
    let h = height / rows as f64;
    let mut cells = Vec::with_capacity(n);
    for &k in counts {
        let w = width / k as f64;
        for _ in 0..k {
            cells.push((w, h));
        }
    }
    let mean_area = cells.iter().map(|&(w, ch)| w * ch).sum::<f64>() / n as f64;
    let mut cost = 0.0;
    for (w, ch) in cells {
        cost += ((w / ch) / TARGET_ASPECT).ln().abs();
        if w < MIN_CELL_WIDTH {
            cost += 4.0 * (MIN_CELL_WIDTH / w - 1.0);
        }
        if ch < MIN_CELL_HEIGHT {
            cost += 4.0 * (MIN_CELL_HEIGHT / ch - 1.0);
        }
        cost += 0.5 * ((w * ch) / mean_area).ln().abs();
    }
    cost / n as f64
}

/// Optimal row counts for `n` panes inside a `width x height` room (`paneGrid.js:gridShape`).
/// Ties within `1e-9` go to fewer rows.
pub fn grid_shape(n: usize, width: f64, height: f64) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![1];
    }
    let w = if width > 0.0 { width } else { 1600.0 };
    let h = if height > 0.0 { height } else { 900.0 };
    let mut best_counts = Vec::new();
    let mut best_cost = f64::INFINITY;
    for rows in 1..=n {
        let counts = row_counts(n, rows);
        let cost = shape_cost(&counts, w, h);
        if best_counts.is_empty() || cost < best_cost - 1e-9 {
            best_counts = counts;
            best_cost = cost;
        }
    }
    best_counts
}

pub fn share_of(children_len: usize, sizes: &[f64], index: usize) -> f64 {
    if children_len == 0 {
        return 0.0;
    }
    if sizes.len() == children_len
        && let Some(&s) = sizes.get(index)
        && s.is_finite()
        && s > 0.0
    {
        return s;
    }
    100.0 / children_len as f64
}

pub fn shares_of(children_len: usize, sizes: &[f64]) -> Vec<f64> {
    (0..children_len)
        .map(|i| share_of(children_len, sizes, i))
        .collect()
}

pub fn is_near_grid(tree: Option<&PaneNode>) -> bool {
    let Some(node) = tree else {
        return true;
    };
    let near = |a: f64, b: f64| (a - b).abs() <= 2.0;
    let equal = |children: &[PaneNode], sizes: &[f64]| {
        let k = children.len();
        k > 0 && (0..k).all(|i| near(share_of(k, sizes, i), 100.0 / k as f64))
    };
    match node {
        PaneNode::Leaf(_) => true,
        PaneNode::Split {
            dir,
            children,
            sizes,
            ..
        } => {
            if !equal(children, sizes) {
                return false;
            }
            if children.iter().all(|c| matches!(c, PaneNode::Leaf(_))) {
                return true;
            }
            if *dir != SplitDir::Col {
                return false;
            }
            children.iter().all(|c| match c {
                PaneNode::Leaf(_) => true,
                PaneNode::Split {
                    dir: SplitDir::Row,
                    children: rc,
                    sizes: rs,
                    ..
                } => equal(rc, rs) && rc.iter().all(|l| matches!(l, PaneNode::Leaf(_))),
                PaneNode::Split { .. } => false,
            })
        }
    }
}

pub fn build_grid_tree<F>(
    leaves: Vec<PaneLeaf>,
    counts: &[usize],
    mut next_split_id: F,
) -> Option<PaneNode>
where
    F: FnMut() -> String,
{
    if leaves.is_empty() {
        return None;
    }
    if leaves.len() == 1 {
        return leaves.into_iter().next().map(PaneNode::Leaf);
    }
    let mut iter = leaves.into_iter().map(PaneNode::Leaf);
    let mut rows: Vec<Vec<PaneNode>> = Vec::with_capacity(counts.len());
    for &k in counts {
        let row: Vec<PaneNode> = iter.by_ref().take(k).collect();
        if !row.is_empty() {
            rows.push(row);
        }
    }
    let tail: Vec<PaneNode> = iter.collect();
    if !tail.is_empty() {
        rows.push(tail);
    }
    if rows.len() == 1 {
        return Some(PaneNode::equal_split(
            next_split_id(),
            SplitDir::Row,
            rows.remove(0),
        ));
    }
    let col_children = rows
        .into_iter()
        .map(|mut r| {
            if r.len() == 1 {
                r.remove(0)
            } else {
                PaneNode::equal_split(next_split_id(), SplitDir::Row, r)
            }
        })
        .collect();
    Some(PaneNode::equal_split(
        next_split_id(),
        SplitDir::Col,
        col_children,
    ))
}

pub fn tree_leaves<P>(tree: Option<&PaneNode>, mut keep: P) -> Vec<&PaneLeaf>
where
    P: FnMut(&PaneLeaf) -> bool,
{
    fn walk<'a, P: FnMut(&PaneLeaf) -> bool>(
        node: &'a PaneNode,
        keep: &mut P,
        out: &mut Vec<&'a PaneLeaf>,
    ) {
        match node {
            PaneNode::Leaf(leaf) => {
                if keep(leaf) {
                    out.push(leaf);
                }
            }
            PaneNode::Split { children, .. } => {
                for child in children {
                    walk(child, keep, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    if let Some(root) = tree {
        walk(root, &mut keep, &mut out);
    }
    out
}

pub fn find_leaf<'a>(tree: Option<&'a PaneNode>, id: &str) -> Option<&'a PaneLeaf> {
    tree_leaves(tree, |leaf| leaf.id == id).into_iter().next()
}

pub fn without_leaves(node: &PaneNode, ids: &HashSet<String>) -> Option<PaneNode> {
    match node {
        PaneNode::Leaf(leaf) => (!ids.contains(&leaf.id)).then(|| PaneNode::Leaf(leaf.clone())),
        PaneNode::Split {
            id,
            dir,
            children,
            sizes,
        } => {
            let mut kept = Vec::with_capacity(children.len());
            for (i, child) in children.iter().enumerate() {
                if let Some(k) = without_leaves(child, ids) {
                    kept.push((k, share_of(children.len(), sizes, i)));
                }
            }
            if kept.is_empty() {
                return None;
            }
            if kept.len() == 1 {
                return kept.into_iter().next().map(|(n, _)| n);
            }
            if kept.len() == children.len()
                && kept.iter().enumerate().all(|(i, (n, _))| n == &children[i])
            {
                return Some(node.clone());
            }
            let total: f64 = kept.iter().map(|(_, s)| *s).sum();
            let next_sizes = kept
                .iter()
                .map(|(_, s)| {
                    if total > 0.0 {
                        (s / total) * 100.0
                    } else {
                        100.0 / kept.len() as f64
                    }
                })
                .collect();
            let next_children = kept.into_iter().map(|(n, _)| n).collect();
            Some(PaneNode::Split {
                id: id.clone(),
                dir: *dir,
                children: next_children,
                sizes: next_sizes,
            })
        }
    }
}

fn replace_leaf(node: &PaneNode, id: &str, replacement: &PaneNode) -> PaneNode {
    match node {
        PaneNode::Leaf(leaf) => {
            if leaf.id == id {
                replacement.clone()
            } else {
                node.clone()
            }
        }
        PaneNode::Split {
            id: split_id,
            dir,
            children,
            sizes,
        } => {
            let next_children: Vec<PaneNode> = children
                .iter()
                .map(|c| replace_leaf(c, id, replacement))
                .collect();
            PaneNode::Split {
                id: split_id.clone(),
                dir: *dir,
                children: next_children,
                sizes: sizes.clone(),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropZone {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl DropZone {
    pub fn at(x: f32, y: f32, allow_swap: bool) -> Option<Self> {
        if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            return None;
        }
        if allow_swap && (0.3..=0.7).contains(&x) && (0.3..=0.7).contains(&y) {
            return Some(Self::Center);
        }
        [
            (Self::Left, x),
            (Self::Right, 1. - x),
            (Self::Top, y),
            (Self::Bottom, 1. - y),
        ]
        .into_iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(zone, _)| zone)
    }
}

pub fn reposition_leaf(
    tree: &PaneNode,
    from: &str,
    to: &str,
    zone: DropZone,
    split_id: String,
) -> Option<PaneNode> {
    if from == to {
        return None;
    }
    let source = find_leaf(Some(tree), from)?;
    let target = find_leaf(Some(tree), to)?;
    if zone == DropZone::Center {
        fn swap(node: &mut PaneNode, a: &PaneLeaf, b: &PaneLeaf) {
            match node {
                PaneNode::Leaf(leaf) if leaf.id == a.id => *leaf = b.clone(),
                PaneNode::Leaf(leaf) if leaf.id == b.id => *leaf = a.clone(),
                PaneNode::Leaf(_) => (),
                PaneNode::Split { children, .. } => {
                    children.iter_mut().for_each(|child| swap(child, a, b))
                }
            }
        }
        let mut next = tree.clone();
        swap(&mut next, source, target);
        return Some(next);
    }
    let rest = without_leaves(tree, &HashSet::from([from.to_owned()]))?;
    let dir = if matches!(zone, DropZone::Left | DropZone::Right) {
        SplitDir::Row
    } else {
        SplitDir::Col
    };
    let mut children = vec![
        PaneNode::Leaf(source.clone()),
        PaneNode::Leaf(target.clone()),
    ];
    if matches!(zone, DropZone::Right | DropZone::Bottom) {
        children.reverse();
    }
    Some(replace_leaf(
        &rest,
        to,
        &PaneNode::split(split_id, dir, children, vec![50., 50.]),
    ))
}

/// Splits leaf `target_id` in `dir`, placing `new_leaf` immediately after `target_id`.
pub fn split_leaf(
    node: &PaneNode,
    target_id: &str,
    dir: SplitDir,
    new_leaf: PaneLeaf,
    split_id: impl Into<String>,
) -> PaneNode {
    let split_node = if let Some(target) = find_leaf(Some(node), target_id) {
        PaneNode::split(
            split_id,
            dir,
            vec![PaneNode::Leaf(target.clone()), PaneNode::Leaf(new_leaf)],
            vec![50.0, 50.0],
        )
    } else {
        return node.clone();
    };
    replace_leaf(node, target_id, &split_node)
}

/// Re-tiles one view's leaves (`in_view`) inside `tree` for a `room_w x room_h` room,
/// leaving leaves of other worktrees and editor/browser tabs untouched (`paneGrid.js:tileTree`).
pub fn tile_tree<P, F>(
    tree: Option<&PaneNode>,
    mut in_view: P,
    extra: Option<PaneLeaf>,
    room_w: f64,
    room_h: f64,
    mut next_split_id: F,
) -> Option<PaneNode>
where
    P: FnMut(&PaneLeaf) -> bool,
    F: FnMut() -> String,
{
    let own: Vec<PaneLeaf> = tree_leaves(tree, &mut in_view)
        .into_iter()
        .cloned()
        .collect();
    let mut leaves = own.clone();
    if let Some(extra_leaf) = extra {
        leaves.push(extra_leaf);
    }
    if leaves.is_empty() {
        return tree.cloned();
    }
    let counts = grid_shape(leaves.len(), room_w, room_h);
    let grid = build_grid_tree(leaves, &counts, &mut next_split_id)?;
    if own.is_empty() {
        return Some(match tree {
            Some(existing) => PaneNode::split(
                next_split_id(),
                SplitDir::Row,
                vec![existing.clone(), grid],
                vec![50.0, 50.0],
            ),
            None => grid,
        });
    }
    let others: HashSet<String> = own.iter().skip(1).map(|l| l.id.clone()).collect();
    let pruned = tree.and_then(|root| without_leaves(root, &others))?;
    Some(replace_leaf(&pruned, &own[0].id, &grid))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridLen {
    pub p: f64,
    pub d: f64,
}

impl GridLen {
    pub const fn new(p: f64, d: f64) -> Self {
        Self { p, d }
    }

    pub fn plus(self, other: Self) -> Self {
        Self::new(self.p + other.p, self.d + other.d)
    }

    pub fn times(self, factor: f64) -> Self {
        Self::new(self.p * factor, self.d * factor)
    }

    pub fn to_px(self, side_px: f64, divider_thickness_px: f64) -> f64 {
        (self.p / 100.0) * side_px + self.d * divider_thickness_px
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridRect {
    pub x: GridLen,
    pub y: GridLen,
    pub w: GridLen,
    pub h: GridLen,
}

impl GridRect {
    pub fn to_px(self, room_w: f64, room_h: f64, divider_px: f64) -> PxRect {
        PxRect {
            left: self.x.to_px(room_w, divider_px),
            top: self.y.to_px(room_h, divider_px),
            width: self.w.to_px(room_w, divider_px).max(0.0),
            height: self.h.to_px(room_h, divider_px).max(0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PxRect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl PxRect {
    pub fn right(self) -> f64 {
        self.left + self.width
    }

    pub fn bottom(self) -> f64 {
        self.top + self.height
    }

    pub fn center_x(self) -> f64 {
        self.left + self.width * 0.5
    }

    pub fn center_y(self) -> f64 {
        self.top + self.height * 0.5
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridTile {
    pub leaf: PaneLeaf,
    pub rect: GridRect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridDivider {
    pub key: String,
    pub split_id: String,
    pub index: usize,
    pub dir: SplitDir,
    pub rect: GridRect,
    pub split_rect: GridRect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GridLayout {
    pub tiles: Vec<GridTile>,
    pub dividers: Vec<GridDivider>,
}

/// Flattens a `PaneNode` into positioned tiles and dividers (`paneGrid.js:gridLayout`).
pub fn grid_layout(tree: Option<&PaneNode>) -> GridLayout {
    fn walk(
        node: &PaneNode,
        rect: GridRect,
        path: &str,
        tiles: &mut Vec<GridTile>,
        dividers: &mut Vec<GridDivider>,
    ) {
        match node {
            PaneNode::Leaf(leaf) => tiles.push(GridTile {
                leaf: leaf.clone(),
                rect,
            }),
            PaneNode::Split {
                id,
                dir,
                children,
                sizes,
            } => {
                let n = children.len();
                if n == 0 {
                    return;
                }
                let is_row = *dir == SplitDir::Row;
                let total = if is_row { rect.w } else { rect.h };
                let room = GridLen::new(total.p, total.d - (n - 1) as f64);
                let sum: f64 = (0..n).map(|i| share_of(n, sizes, i)).sum();
                let mut at = if is_row { rect.x } else { rect.y };
                for (i, child) in children.iter().enumerate() {
                    let size = room.times(share_of(n, sizes, i) / sum);
                    let child_rect = if is_row {
                        GridRect {
                            x: at,
                            w: size,
                            ..rect
                        }
                    } else {
                        GridRect {
                            y: at,
                            h: size,
                            ..rect
                        }
                    };
                    let child_path = format!("{path}.{i}");
                    walk(child, child_rect, &child_path, tiles, dividers);
                    at = at.plus(size);
                    if i + 1 < n {
                        let line = GridLen::new(0.0, 1.0);
                        let div_rect = if is_row {
                            GridRect {
                                x: at,
                                w: line,
                                ..rect
                            }
                        } else {
                            GridRect {
                                y: at,
                                h: line,
                                ..rect
                            }
                        };
                        let key_prefix = if id.is_empty() { path } else { id.as_str() };
                        dividers.push(GridDivider {
                            key: format!("{key_prefix}:{i}"),
                            split_id: id.clone(),
                            index: i,
                            dir: *dir,
                            rect: div_rect,
                            split_rect: rect,
                        });
                        at = at.plus(line);
                    }
                }
            }
        }
    }

    let mut tiles = Vec::new();
    let mut dividers = Vec::new();
    if let Some(root) = tree {
        walk(
            root,
            GridRect {
                x: GridLen::new(0.0, 0.0),
                y: GridLen::new(0.0, 0.0),
                w: GridLen::new(100.0, 0.0),
                h: GridLen::new(100.0, 0.0),
            },
            "s",
            &mut tiles,
            &mut dividers,
        );
    }
    GridLayout { tiles, dividers }
}

/// Keyboard arrow resize on divider `index` (`±2%`, each neighbor keeps `>= 5%`, `PaneGrid.vue:68-87`).
pub fn resize_divider_key(
    children_len: usize,
    sizes: &[f64],
    index: usize,
    forward: bool,
) -> Option<Vec<f64>> {
    if index + 1 >= children_len {
        return None;
    }
    let mut next = shares_of(children_len, sizes);
    let step = if forward {
        DIVIDER_KEY_STEP_PCT
    } else {
        -DIVIDER_KEY_STEP_PCT
    };
    let a = next[index] + step;
    let b = next[index + 1] - step;
    if a < DIVIDER_MIN_PCT || b < DIVIDER_MIN_PCT {
        return None;
    }
    next[index] = a;
    next[index + 1] = b;
    Some(next)
}

/// Pointer drag on divider `index` (`PaneGrid.vue:91-127`).
/// Adjusts ONLY the adjacent pair `(index, index + 1)` while preserving `sizes[index] + sizes[index + 1]`.
pub fn resize_divider_drag(
    children_len: usize,
    sizes: &[f64],
    index: usize,
    dir: SplitDir,
    split_inner_px: f64,
    delta_px: f64,
) -> Option<Vec<f64>> {
    if index + 1 >= children_len || !split_inner_px.is_finite() || split_inner_px <= 0.0 {
        return None;
    }
    let mut next = shares_of(children_len, sizes);
    let sum: f64 = next.iter().sum();
    if !sum.is_finite() || sum <= 0.0 {
        return None;
    }
    let a0 = next[index];
    let pair_sum = a0 + next[index + 1];
    let min_px = match dir {
        SplitDir::Row => MIN_PANE_WIDTH,
        SplitDir::Col => MIN_PANE_HEIGHT,
    };
    let min_share = (pair_sum * 0.5).min((min_px / split_inner_px) * sum);
    let a = (a0 + (delta_px / split_inner_px) * sum).clamp(min_share, pair_sum - min_share);
    next[index] = a;
    next[index + 1] = pair_sum - a;
    Some(next)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

/// Geometric `Alt+Arrow` neighbor selection (`App.vue:5840-5884`).
pub fn move_focus(panes: &[(String, PxRect)], active_id: &str, dir: FocusDir) -> Option<String> {
    let (_, cur) = panes.iter().find(|(id, _)| id == active_id)?;
    let mut best_id: Option<&str> = None;
    let mut best_score = f64::INFINITY;
    for (id, r) in panes {
        if id == active_id {
            continue;
        }
        let (ok, dist, off) = match dir {
            FocusDir::Left => (
                r.right() <= cur.left + 2.0,
                cur.left - r.right(),
                (r.center_y() - cur.center_y()).abs(),
            ),
            FocusDir::Right => (
                r.left >= cur.right() - 2.0,
                r.left - cur.right(),
                (r.center_y() - cur.center_y()).abs(),
            ),
            FocusDir::Up => (
                r.bottom() <= cur.top + 2.0,
                cur.top - r.bottom(),
                (r.center_x() - cur.center_x()).abs(),
            ),
            FocusDir::Down => (
                r.top >= cur.bottom() - 2.0,
                r.top - cur.bottom(),
                (r.center_x() - cur.center_x()).abs(),
            ),
        };
        if !ok {
            continue;
        }
        let score = dist * 4.0 + off;
        if score < best_score {
            best_score = score;
            best_id = Some(id.as_str());
        }
    }
    best_id.map(str::to_owned)
}

// --- paneViews.js + gridReturnLeaf ---------------------------------------------------

pub fn norm_path(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .replace('\\', "/")
        .to_lowercase()
}

pub fn leaf_view_path(leaf: &PaneLeaf) -> &str {
    if let Some(wt) = leaf.worktree_path.as_deref()
        && !wt.is_empty()
    {
        return wt;
    }
    leaf.view_path.as_deref().unwrap_or("")
}

pub fn view_key(leaf: &PaneLeaf, project_cwd: &str) -> String {
    let key = norm_path(leaf_view_path(leaf));
    if !key.is_empty() && key != norm_path(project_cwd) {
        key
    } else {
        String::new()
    }
}

pub fn is_tab_leaf(leaf: &PaneLeaf) -> bool {
    matches!(leaf.kind, LeafKind::Editor | LeafKind::Browser) || leaf.live_share_page
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeViewSummary {
    pub key: String,
    pub ids: BTreeSet<String>,
}

/// Views of a workspace tree, with `""` (the project folder) always first,
/// followed by worktree copies sorted lexicographically (`paneViews.js:treeViews`).
pub fn tree_views(tree: Option<&PaneNode>, project_cwd: &str) -> Vec<TreeViewSummary> {
    let mut by_key: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for leaf in tree_leaves(tree, |_| true) {
        by_key
            .entry(view_key(leaf, project_cwd))
            .or_default()
            .insert(leaf.id.clone());
    }
    by_key
        .into_iter()
        .map(|(key, ids)| TreeViewSummary { key, ids })
        .collect()
}

pub fn active_view_key(views: &[TreeViewSummary], active_id: Option<&str>) -> String {
    if let Some(id) = active_id
        && let Some(view) = views.iter().find(|v| v.ids.contains(id))
    {
        return view.key.clone();
    }
    views.first().map(|v| v.key.clone()).unwrap_or_default()
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrunedSplitMeta {
    pub source_split_id: String,
    pub source_indexes: Vec<usize>,
}

/// Prunes `node` to leaves accepted by `keep`, recording `PrunedSplitMeta` by pruned split ID
/// when a split keeps only a subset of `>= 2` children (`paneViews.js:pruneTree`).
pub fn prune_tree<P>(
    node: &PaneNode,
    keep: &mut P,
    sources: &mut BTreeMap<String, PrunedSplitMeta>,
) -> Option<PaneNode>
where
    P: FnMut(&PaneLeaf) -> bool,
{
    match node {
        PaneNode::Leaf(leaf) => keep(leaf).then(|| PaneNode::Leaf(leaf.clone())),
        PaneNode::Split {
            id,
            dir,
            children,
            sizes,
        } => {
            let mut kept: Vec<(PaneNode, usize)> = Vec::new();
            for (i, child) in children.iter().enumerate() {
                if let Some(k) = prune_tree(child, keep, sources) {
                    kept.push((k, i));
                }
            }
            if kept.is_empty() {
                return None;
            }
            if kept.len() == 1 {
                return kept.into_iter().next().map(|(n, _)| n);
            }
            if kept.len() == children.len()
                && kept.iter().enumerate().all(|(i, (n, _))| n == &children[i])
            {
                return Some(node.clone());
            }
            let size_at = |idx: usize| sizes.get(idx).copied().unwrap_or(0.0);
            let total: f64 = kept.iter().map(|(_, idx)| size_at(*idx)).sum();
            let view_sizes: Vec<f64> = kept
                .iter()
                .map(|(_, idx)| {
                    if total > 0.0 {
                        (size_at(*idx) / total) * 100.0
                    } else {
                        100.0 / kept.len() as f64
                    }
                })
                .collect();
            let source_indexes: Vec<usize> = kept.iter().map(|(_, idx)| *idx).collect();
            let pruned_id = format!("{id}::view");
            sources.insert(
                pruned_id.clone(),
                PrunedSplitMeta {
                    source_split_id: id.clone(),
                    source_indexes,
                },
            );
            Some(PaneNode::Split {
                id: pruned_id,
                dir: *dir,
                children: kept.into_iter().map(|(n, _)| n).collect(),
                sizes: view_sizes,
            })
        }
    }
}

/// Propagates divider share edits from a pruned view split back into its source split
/// in the workspace tree (`paneViews.js:writeViewSizes`).
pub fn write_view_sizes(tree: &mut PaneNode, meta: &PrunedSplitMeta, view_sizes: &[f64]) -> bool {
    match tree {
        PaneNode::Leaf(_) => false,
        PaneNode::Split {
            id,
            children,
            sizes,
            ..
        } => {
            if id == &meta.source_split_id {
                let k = children.len();
                if k == 0 || meta.source_indexes.len() != view_sizes.len() {
                    return false;
                }
                let mut next = if sizes.len() == k {
                    sizes.clone()
                } else {
                    vec![100.0 / k as f64; k]
                };
                let room: f64 = meta
                    .source_indexes
                    .iter()
                    .map(|&idx| next.get(idx).copied().unwrap_or(0.0))
                    .sum();
                let own: f64 = view_sizes.iter().sum();
                if !room.is_finite() || room <= 0.0 || !own.is_finite() || own <= 0.0 {
                    return false;
                }
                for (&src_idx, &v_size) in meta.source_indexes.iter().zip(view_sizes.iter()) {
                    if let Some(slot) = next.get_mut(src_idx) {
                        *slot = (v_size / own) * room;
                    }
                }
                *sizes = next;
                return true;
            }
            children
                .iter_mut()
                .any(|c| write_view_sizes(c, meta, view_sizes))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShownTab {
    #[default]
    Terminal,
    Leaf(String),
}

impl ShownTab {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminal)
    }

    pub fn leaf_id(&self) -> Option<&str> {
        match self {
            Self::Terminal => None,
            Self::Leaf(id) => Some(id.as_str()),
        }
    }
}

pub fn shown_tab(
    active_id: Option<&str>,
    preferred_view: &ShownTab,
    view_ids: &BTreeSet<String>,
    has_grid: bool,
    tabs: &[PaneLeaf],
) -> ShownTab {
    let is_tab_id = |id: &str| tabs.iter().any(|l| l.id == id);
    if let Some(act) = active_id
        && view_ids.contains(act)
    {
        if !is_tab_id(act) {
            return ShownTab::Terminal;
        }
        return if preferred_view.is_terminal() && !has_grid {
            ShownTab::Terminal
        } else {
            ShownTab::Leaf(act.to_owned())
        };
    }
    if let Some(id) = preferred_view.leaf_id()
        && is_tab_id(id)
    {
        return ShownTab::Leaf(id.to_owned());
    }
    ShownTab::Terminal
}

pub fn add_tab_leaf(
    tree: Option<PaneNode>,
    tab_leaf: PaneLeaf,
    split_id: impl Into<String>,
) -> PaneNode {
    match tree {
        Some(existing) => PaneNode::split(
            split_id,
            SplitDir::Row,
            vec![existing, PaneNode::Leaf(tab_leaf)],
            vec![50.0, 50.0],
        ),
        None => PaneNode::Leaf(tab_leaf),
    }
}

/// Last active grid leaf in `from`'s view (`grid_id`), else first non-tab leaf in that view.
pub fn grid_leaf_for<'a>(
    tree: Option<&'a PaneNode>,
    cwd: &str,
    grid_id: Option<&str>,
    from: &PaneLeaf,
) -> Option<&'a PaneLeaf> {
    let key = view_key(from, cwd);
    let mut first = None;
    let mut last = None;
    for leaf in tree_leaves(tree, |_| true) {
        if is_tab_leaf(leaf) || view_key(leaf, cwd) != key {
            continue;
        }
        if first.is_none() {
            first = Some(leaf);
        }
        if grid_id == Some(leaf.id.as_str()) {
            last = Some(leaf);
        }
    }
    last.or(first)
}

/// Exact `gridReturnLeaf` invariant (`App.vue:1277-1281`):
/// when leaving or closing an editor/browser tab in the current workspace, focus returns to
/// `maximized_id` if a grid leaf in that same view is maximized, otherwise `grid_leaf_for`.
pub fn grid_return_leaf<'a>(
    tree: Option<&'a PaneNode>,
    cwd: &str,
    grid_id: Option<&str>,
    maximized_id: Option<&str>,
    is_current_workspace: bool,
    from: &PaneLeaf,
) -> Option<&'a PaneLeaf> {
    let grid = grid_leaf_for(tree, cwd, grid_id, from)?;
    if is_current_workspace
        && let Some(max_id) = maximized_id
        && let Some(max_leaf) = find_leaf(tree, max_id)
        && !is_tab_leaf(max_leaf)
        && view_key(max_leaf, cwd) == view_key(grid, cwd)
    {
        return Some(max_leaf);
    }
    Some(grid)
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceViewLayer {
    pub key: String,
    pub tree: Option<PaneNode>,
    pub grid: Option<PaneNode>,
    pub tabs: Vec<PaneLeaf>,
    pub shown: ShownTab,
    pub active: bool,
    pub pruned_sources: BTreeMap<String, PrunedSplitMeta>,
}

#[derive(Clone, Debug, Default)]
pub struct WorkspacePaneState {
    pub cwd: String,
    pub tree: Option<PaneNode>,
    pub active_id: Option<String>,
    pub grid_id: Option<String>,
    pub view: ShownTab,
}

impl WorkspacePaneState {
    pub fn new(cwd: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            tree: None,
            active_id: None,
            grid_id: None,
            view: ShownTab::Terminal,
        }
    }

    pub fn set_active(&mut self, id: impl Into<String>) {
        let id = id.into();
        if let Some(leaf) = find_leaf(self.tree.as_ref(), &id) {
            if is_tab_leaf(leaf) {
                self.view = ShownTab::Leaf(id.clone());
            } else {
                self.grid_id = Some(id.clone());
                self.view = ShownTab::Terminal;
            }
            self.active_id = Some(id);
        }
    }

    /// Computes all per-worktree view layers (`paneViews.js:workspaceViews`).
    pub fn layers(&self) -> Vec<WorkspaceViewLayer> {
        let Some(root) = self.tree.as_ref() else {
            return Vec::new();
        };
        let views = tree_views(Some(root), &self.cwd);
        let active_key = active_view_key(&views, self.active_id.as_deref());
        let single_view = views.len() == 1;
        views
            .into_iter()
            .map(|v| {
                let tabs: Vec<PaneLeaf> = tree_leaves(Some(root), |leaf| {
                    v.ids.contains(&leaf.id) && is_tab_leaf(leaf)
                })
                .into_iter()
                .cloned()
                .collect();
                let mut pruned_sources = BTreeMap::new();
                let grid = prune_tree(
                    root,
                    &mut |leaf| v.ids.contains(&leaf.id) && !is_tab_leaf(leaf),
                    &mut pruned_sources,
                );
                let mut full_sources = BTreeMap::new();
                let view_tree = if single_view {
                    Some(root.clone())
                } else {
                    prune_tree(
                        root,
                        &mut |leaf| v.ids.contains(&leaf.id),
                        &mut full_sources,
                    )
                };
                let shown = shown_tab(
                    self.active_id.as_deref(),
                    &self.view,
                    &v.ids,
                    grid.is_some(),
                    &tabs,
                );
                WorkspaceViewLayer {
                    key: if v.key.is_empty() {
                        ":main".to_owned()
                    } else {
                        v.key.clone()
                    },
                    tree: view_tree,
                    grid,
                    tabs,
                    shown,
                    active: v.key == active_key,
                    pruned_sources,
                }
            })
            .collect()
    }

    /// Implements `showViewTab` (`App.vue:1250-1272`).
    pub fn show_view_tab(
        &mut self,
        target: ShownTab,
        maximized_id: Option<&str>,
        is_current_workspace: bool,
    ) {
        let Some(root) = self.tree.as_ref() else {
            return;
        };
        match target {
            ShownTab::Terminal => {
                let ret = self
                    .active_id
                    .as_deref()
                    .and_then(|id| find_leaf(Some(root), id))
                    .filter(|leaf| is_tab_leaf(leaf))
                    .and_then(|active_tab| {
                        grid_return_leaf(
                            Some(root),
                            &self.cwd,
                            self.grid_id.as_deref(),
                            maximized_id,
                            is_current_workspace,
                            active_tab,
                        )
                    })
                    .map(|l| l.id.clone());
                self.view = ShownTab::Terminal;
                if let Some(id) = ret {
                    self.active_id = Some(id);
                }
            }
            ShownTab::Leaf(id) => {
                let is_valid_tab = find_leaf(Some(root), &id).is_some_and(is_tab_leaf);
                if !is_valid_tab {
                    return;
                }
                self.view = ShownTab::Leaf(id.clone());
                self.active_id = Some(id);
            }
        }
    }

    /// Closes leaf `leaf_id` and updates `active_id`, `view`, and `maximized_id`
    /// (`App.vue:2842-2855`).
    pub fn close_leaf(
        &mut self,
        leaf_id: &str,
        maximized_id: &mut Option<String>,
        is_current_workspace: bool,
    ) {
        let Some(root) = self.tree.as_ref() else {
            return;
        };
        let Some(closing) = find_leaf(Some(root), leaf_id).cloned() else {
            return;
        };
        if maximized_id.as_deref() == Some(leaf_id) {
            *maximized_id = None;
        }
        if self.grid_id.as_deref() == Some(leaf_id) {
            self.grid_id = None;
        }
        let mut drop_set = HashSet::new();
        drop_set.insert(leaf_id.to_owned());
        let next = without_leaves(root, &drop_set);
        if let Some(ref next_tree) = next {
            if self.active_id.as_deref() == Some(leaf_id) {
                let grid_ret = grid_return_leaf(
                    Some(next_tree),
                    &self.cwd,
                    self.grid_id.as_deref(),
                    maximized_id.as_deref(),
                    is_current_workspace,
                    &closing,
                )
                .map(|l| l.id.clone());
                let next_active = if let Some(gid) = grid_ret {
                    Some(gid)
                } else {
                    let closing_key = view_key(&closing, &self.cwd);
                    let same_copy =
                        tree_leaves(Some(next_tree), |l| view_key(l, &self.cwd) == closing_key)
                            .first()
                            .map(|l| l.id.clone());
                    same_copy.or_else(|| {
                        tree_leaves(Some(next_tree), |_| true)
                            .first()
                            .map(|l| l.id.clone())
                    })
                };
                self.active_id = next_active;
                self.view = ShownTab::Terminal;
            } else if self.view.leaf_id() == Some(leaf_id) {
                self.view = ShownTab::Terminal;
            }
        } else {
            self.active_id = None;
            self.view = ShownTab::Terminal;
        }
        self.tree = next;
    }
}

// --- Editor preview/pinned/dirty tab strip model (`EditorPane.vue` + `ViewTabs.vue`) ---

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorTabEntry {
    pub path: String,
    pub pinned: bool,
    pub dirty: bool,
    pub deleted: bool,
    pub failed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditorTabStrip {
    pub files: Vec<EditorTabEntry>,
    pub active_path: Option<String>,
}

impl EditorTabStrip {
    /// Opens `path`. Single-click (`pin == false`) reuses any existing clean unpinned preview tab;
    /// double-click or explicit open (`pin == true`) pins the tab (`EditorPane.vue`).
    pub fn open_file(&mut self, path: impl Into<String>, pin: bool) {
        let path = path.into();
        let norm = norm_path(&path);
        if let Some(existing) = self.files.iter_mut().find(|f| norm_path(&f.path) == norm) {
            if pin {
                existing.pinned = true;
            }
            self.active_path = Some(existing.path.clone());
            return;
        }
        if !pin && let Some(preview) = self.files.iter_mut().find(|f| !f.pinned && !f.dirty) {
            preview.path = path.clone();
            preview.deleted = false;
            preview.failed = false;
            self.active_path = Some(path);
            return;
        }
        self.files.push(EditorTabEntry {
            path: path.clone(),
            pinned: pin,
            dirty: false,
            deleted: false,
            failed: false,
        });
        self.active_path = Some(path);
    }

    pub fn pin(&mut self, path: &str) {
        let norm = norm_path(path);
        if let Some(f) = self.files.iter_mut().find(|f| norm_path(&f.path) == norm) {
            f.pinned = true;
        }
    }

    /// Marks `path` dirty/clean; editing a preview file automatically pins it (`EditorPane.vue`).
    pub fn set_dirty(&mut self, path: &str, dirty: bool) {
        let norm = norm_path(path);
        if let Some(f) = self.files.iter_mut().find(|f| norm_path(&f.path) == norm) {
            f.dirty = dirty;
            if dirty {
                f.pinned = true;
            }
        }
    }

    pub fn accessible_label(entry: &EditorTabEntry, french: bool) -> String {
        let name = entry
            .path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(entry.path.as_str());
        if entry.dirty {
            if french {
                format!("{name} (modifié)")
            } else {
                format!("{name} (unsaved)")
            }
        } else {
            name.to_owned()
        }
    }

    /// Closes `path`; returns `false` if the file is dirty and `force` is `false`.
    pub fn close(&mut self, path: &str, force: bool) -> bool {
        let norm = norm_path(path);
        let Some(idx) = self.files.iter().position(|f| norm_path(&f.path) == norm) else {
            return true;
        };
        if self.files[idx].dirty && !force {
            return false;
        }
        let was_active = self
            .active_path
            .as_deref()
            .is_some_and(|a| norm_path(a) == norm);
        self.files.remove(idx);
        if was_active {
            self.active_path = if self.files.is_empty() {
                None
            } else {
                let next_idx = idx.min(self.files.len() - 1);
                Some(self.files[next_idx].path.clone())
            };
        }
        true
    }
}

#[cfg(test)]
mod drop_tests {
    use super::*;
    #[test]
    fn pane_drop_zones_match_inclusive_center_and_nearest_edge_ties() {
        use DropZone::*;
        for (x, y, swap, expected) in [
            (0.3, 0.7, true, Center),
            (0.7, 0.3, true, Center),
            (0.5, 0.5, false, Left),
            (0.9, 0.5, true, Right),
            (0.5, 0.1, true, Top),
            (0.4, 0.95, true, Bottom),
        ] {
            assert_eq!(DropZone::at(x, y, swap), Some(expected));
        }
        assert_eq!(DropZone::at(f32::NAN, 0.5, true), None);
        let a = PaneLeaf::new("one", LeafKind::Browser).with_view_path("branch");
        let b = PaneLeaf::new("two", LeafKind::Agent).with_worktree("agent-dir");
        let root = PaneNode::split(
            "old",
            SplitDir::Row,
            vec![PaneNode::Leaf(a.clone()), PaneNode::Leaf(b.clone())],
            vec![30., 70.],
        );
        for zone in [Center, Left, Right, Top, Bottom] {
            let next = reposition_leaf(&root, "one", "two", zone, "new".into()).unwrap();
            let PaneNode::Split { dir, sizes, .. } = &next else {
                panic!()
            };
            assert_eq!(
                *dir,
                if matches!(zone, Top | Bottom) {
                    SplitDir::Col
                } else {
                    SplitDir::Row
                }
            );
            assert_eq!(
                sizes,
                if zone == Center {
                    &[30., 70.]
                } else {
                    &[50., 50.]
                }
            );
            assert_eq!(
                tree_leaves(Some(&next), |_| true),
                if matches!(zone, Left | Top) {
                    vec![&a, &b]
                } else {
                    vec![&b, &a]
                }
            );
        }
        assert!(reposition_leaf(&root, "one", "gone", Top, "new".into()).is_none());
    }
}
