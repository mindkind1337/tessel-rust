#[path = "../src/outline_tree.rs"]
mod outline_tree;
use outline_tree::*;
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Case>,
    matches: Vec<MatchCase>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    input: Vec<Symbol>,
    by: Sort,
    query: String,
    highlight: bool,
    collapsed: HashSet<String>,
    tree: Vec<Symbol>,
    parents: Vec<String>,
    all_collapsed: bool,
    rows: Vec<Row>,
    cursors: Vec<Cursor>,
}
#[derive(Deserialize)]
struct Cursor {
    line: usize,
    column: usize,
    path: Vec<String>,
}
#[derive(Deserialize)]
struct MatchCase {
    name: String,
    query: String,
    expected: Option<Vec<[usize; 2]>>,
}

#[test]
fn reference_tree_sort_filter_folds_and_cursor_vectors() {
    let f: Fixture = serde_json::from_str(include_str!("fixtures/outline_tree.json")).unwrap();
    for (i, c) in f.cases.into_iter().enumerate() {
        let mut tree = c.input;
        keyed(&mut tree, "");
        sort_symbols(&mut tree, c.by);
        assert_eq!(tree, c.tree, "tree {i}");
        assert_eq!(parent_keys(&tree), c.parents, "parents {i}");
        assert_eq!(
            all_collapsed(&tree, &c.collapsed),
            c.all_collapsed,
            "folds {i}"
        );
        assert_eq!(
            rows(&tree, &c.collapsed, &c.query, c.highlight),
            c.rows,
            "rows {i}"
        );
        for cursor in c.cursors {
            assert_eq!(
                symbol_path_at(&tree, cursor.line, cursor.column),
                cursor.path,
                "cursor {i}"
            );
        }
    }
    for c in f.matches {
        assert_eq!(
            match_name(&c.name, &c.query),
            c.expected,
            "match {:?} {:?}",
            c.name,
            c.query
        );
    }
}

#[test]
fn keys_survive_line_edits_and_sibling_twins_remain_distinct() {
    let f: Fixture = serde_json::from_str(include_str!("fixtures/outline_tree.json")).unwrap();
    let mut tree = f.cases[0].input.clone();
    keyed(&mut tree, "");
    let before = rows(&tree, &HashSet::new(), "", false);
    fn shift(list: &mut [Symbol]) {
        for s in list {
            s.range.start_line_number += 100;
            s.range.end_line_number += 100;
            shift(&mut s.children);
        }
    }
    shift(&mut tree);
    keyed(&mut tree, "");
    assert_eq!(rows(&tree, &HashSet::new(), "", false), before);
    assert_eq!(
        before.iter().map(|r| &r.key).collect::<HashSet<_>>().len(),
        before.len()
    );
}
