//! Native outline tree behavior, adapted from explorer/outlineTree.js.
//! Coordinates and match spans use Monaco's one-based positions / UTF-16 offsets.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Range {
    pub start_line_number: usize,
    pub start_column: usize,
    pub end_line_number: usize,
    pub end_column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: u8,
    pub range: Range,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default, rename = "selectionRange")]
    pub selection_range: Option<Range>,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub children: Vec<Symbol>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    Position,
    Name,
    Kind,
}

/// Keys depend on ancestry and sibling identity, never line numbers.
pub fn keyed(symbols: &mut [Symbol], parent: &str) {
    let mut seen = HashMap::new();
    for symbol in symbols {
        let base = format!("{}:{}", symbol.kind, symbol.name);
        let count = seen.entry(base.clone()).or_insert(0usize);
        symbol.key = if *count == 0 {
            format!("{parent}/{base}")
        } else {
            format!("{parent}/{base}#{count}")
        };
        *count += 1;
        keyed(&mut symbol.children, &symbol.key);
    }
}

fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    static COLLATOR: std::sync::OnceLock<icu_collator::CollatorBorrowed<'static>> =
        std::sync::OnceLock::new();
    COLLATOR
        .get_or_init(|| {
            let mut prefs = icu_collator::CollatorPreferences::default();
            prefs.numeric_ordering =
                Some(icu_collator::preferences::CollationNumericOrdering::True);
            icu_collator::Collator::try_new(prefs, Default::default())
                .expect("compiled outline collation")
        })
        .compare(a, b)
}

pub fn sort_symbols(symbols: &mut [Symbol], by: Sort) {
    symbols.sort_by(|a, b| match by {
        Sort::Position => a
            .range
            .cmp(&b.range)
            .then_with(|| compare_names(&a.name, &b.name)),
        Sort::Name => compare_names(&a.name, &b.name).then_with(|| a.range.cmp(&b.range)),
        Sort::Kind => a
            .kind
            .cmp(&b.kind)
            .then_with(|| compare_names(&a.name, &b.name)),
    });
    for symbol in symbols {
        sort_symbols(&mut symbol.children, by);
    }
}

pub fn symbol_path_at(symbols: &[Symbol], line: usize, column: usize) -> Vec<String> {
    let mut list = symbols;
    let mut path = Vec::new();
    for _ in 0..200 {
        // Preserve the first sibling when ranges compare equal, as the reference does.
        let mut hit: Option<&Symbol> = None;
        for symbol in list {
            let r = symbol.range;
            if (line, column) >= (r.start_line_number, r.start_column)
                && (line, column) <= (r.end_line_number, r.end_column)
                && hit.is_none_or(|old| old.range < r)
            {
                hit = Some(symbol);
            }
        }
        let Some(symbol) = hit else {
            break;
        };
        path.push(symbol.key.clone());
        list = &symbol.children;
    }
    path
}

pub fn parent_keys(symbols: &[Symbol]) -> Vec<String> {
    let mut out = Vec::new();
    for s in symbols {
        if !s.children.is_empty() {
            out.push(s.key.clone());
            out.extend(parent_keys(&s.children));
        }
    }
    out
}

pub fn all_collapsed(symbols: &[Symbol], collapsed: &HashSet<String>) -> bool {
    symbols
        .iter()
        .all(|s| s.children.is_empty() || collapsed.contains(&s.key))
}

pub fn match_name(name: &str, query: &str) -> Option<Vec<[usize; 2]>> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return Some(Vec::new());
    }
    let n: Vec<u16> = name.to_lowercase().encode_utf16().collect();
    let needle: Vec<u16> = q.encode_utf16().collect();
    if let Some(at) = n.windows(needle.len()).position(|w| w == needle) {
        return Some(vec![[at, at + needle.len()]]);
    }
    let mut out: Vec<[usize; 2]> = Vec::new();
    let mut start = 0;
    for ch in q.chars().filter(|c| *c != ' ') {
        let mut buf = [0u16; 2];
        let units = ch.encode_utf16(&mut buf);
        let offset = n
            .get(start..)?
            .windows(units.len())
            .position(|w| w == units)?;
        let j = start + offset;
        // Deliberately follows the JS source's j+1 UTF-16 highlighting convention.
        if let Some(last) = out.last_mut().filter(|r| r[1] == j) {
            last[1] = j + 1;
        } else {
            out.push([j, j + 1]);
        }
        start = j + 1;
    }
    Some(out)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub key: String,
    pub depth: usize,
    pub parent: Option<String>,
    pub has_children: bool,
    pub open: bool,
    pub matches: Vec<[usize; 2]>,
}

/// Convert the reference provider's UTF-16 spans to whole UTF-8 characters for painting.
pub fn highlight_ranges(name: &str, matches: &[[usize; 2]]) -> Vec<std::ops::Range<usize>> {
    if matches.is_empty() {
        return Vec::new();
    }
    let mut output: Vec<std::ops::Range<usize>> = Vec::new();
    let mut utf16 = 0;
    let mut spans = matches.iter().peekable();
    for (byte, c) in name.char_indices() {
        while spans.peek().is_some_and(|span| span[1] <= utf16) {
            spans.next();
        }
        let Some(span) = spans.peek() else { break };
        utf16 += c.len_utf16();
        if span[0] < utf16 {
            let end = byte + c.len_utf8();
            if let Some(last) = output.last_mut().filter(|last| last.end == byte) {
                last.end = end;
            } else {
                output.push(byte..end);
            }
        }
    }
    output
}

/// Borrow the original symbols so both views can navigate the ranges of filtered rows.
pub struct VisibleRow<'a> {
    pub symbol: &'a Symbol,
    pub row: Row,
}

type MatchInfo = HashMap<String, (Option<Vec<[usize; 2]>>, bool)>;

pub fn rows(
    symbols: &[Symbol],
    collapsed: &HashSet<String>,
    query: &str,
    highlight: bool,
) -> Vec<Row> {
    visible_rows(symbols, collapsed, query, highlight)
        .into_iter()
        .map(|entry| entry.row)
        .collect()
}

pub fn visible_rows<'a>(
    symbols: &'a [Symbol],
    collapsed: &HashSet<String>,
    query: &str,
    highlight: bool,
) -> Vec<VisibleRow<'a>> {
    let query = query.trim();
    let filtering = !query.is_empty();
    fn scan(list: &[Symbol], query: &str, filtering: bool, info: &mut MatchInfo) -> bool {
        let mut any = false;
        for s in list {
            let matches = if filtering {
                match_name(&s.name, query)
            } else {
                Some(Vec::new())
            };
            let child = scan(&s.children, query, filtering, info);
            let keep = !filtering || matches.is_some() || child;
            info.insert(s.key.clone(), (matches, keep));
            any |= keep;
        }
        any
    }
    struct Walk<'a, 's> {
        collapsed: &'a HashSet<String>,
        filter: bool,
        info: MatchInfo,
        out: Vec<VisibleRow<'s>>,
    }
    impl<'s> Walk<'_, 's> {
        fn visit(&mut self, list: &'s [Symbol], depth: usize, parent: Option<&str>) {
            for s in list {
                let info = self.info.get(&s.key);
                if self.filter && info.is_some_and(|(_, keep)| !keep) {
                    continue;
                }
                let has_children = !s.children.is_empty();
                let open = has_children && (self.filter || !self.collapsed.contains(&s.key));
                self.out.push(VisibleRow {
                    symbol: s,
                    row: Row {
                        key: s.key.clone(),
                        depth,
                        parent: parent.map(str::to_owned),
                        has_children,
                        open,
                        matches: info.and_then(|(m, _)| m.clone()).unwrap_or_default(),
                    },
                });
                if open {
                    self.visit(&s.children, depth + 1, Some(&s.key));
                }
            }
        }
    }
    let mut walk = Walk {
        collapsed,
        filter: filtering && !highlight,
        info: HashMap::new(),
        out: Vec::new(),
    };
    if filtering {
        scan(symbols, query, true, &mut walk.info);
    }
    walk.visit(symbols, 0, None);
    walk.out
}
