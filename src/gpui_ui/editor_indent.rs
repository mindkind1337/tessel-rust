//! Indentation detection and edits for the native source editor.
//! Detection and tab-stop rules adapted from Monaco 0.57.0 (Microsoft, MIT).
//! See THIRD_PARTY_NOTICES.md. Language-specific automatic indentation is separate.
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Options {
    pub size: usize,
    pub spaces: bool,
}

impl Options {
    pub fn detect(text: &str) -> Self {
        let mut tabs = 0;
        let mut spaces = 0;
        let mut scores = [0usize; 9];
        let mut previous = "";
        let mut previous_indent = 0;
        for line in text.lines().take(10_000) {
            let indent = line
                .bytes()
                .take_while(|b| matches!(b, b' ' | b'\t'))
                .count();
            if indent == line.len() {
                continue;
            }
            let tab_count = line[..indent].bytes().filter(|b| *b == b'\t').count();
            if tab_count > 0 {
                tabs += 1;
            } else if indent > 1 {
                spaces += 1;
            }
            let common = previous.as_bytes()[..previous_indent]
                .iter()
                .zip(&line.as_bytes()[..indent])
                .take_while(|(a, b)| a == b)
                .count();
            let a = &previous.as_bytes()[common..previous_indent];
            let b = &line.as_bytes()[common..indent];
            let a_spaces = a.iter().filter(|c| **c == b' ').count();
            let b_spaces = b.iter().filter(|c| **c == b' ').count();
            let a_tabs = a.len() - a_spaces;
            let b_tabs = b.len() - b_spaces;
            let mut difference = 0;
            let mut alignment = false;
            if !(a_spaces > 0 && a_tabs > 0 || b_spaces > 0 && b_tabs > 0) {
                let tab_diff = a_tabs.abs_diff(b_tabs);
                let space_diff = a_spaces.abs_diff(b_spaces);
                if tab_diff == 0 {
                    difference = space_diff;
                    alignment = space_diff > 0
                        && b_spaces > 0
                        && previous.encode_utf16().nth(b_spaces - 1) == Some(32)
                        && line.encode_utf16().nth(b_spaces).is_some_and(|c| c != 32)
                        && previous.ends_with(',');
                } else if space_diff % tab_diff == 0 {
                    difference = space_diff / tab_diff;
                }
            }
            if alignment && difference != 4 {
                continue;
            }
            if difference <= 8 {
                scores[difference] += 1;
            }
            previous = line;
            previous_indent = indent;
        }
        let mut result = Self {
            size: 4,
            spaces: tabs <= spaces,
        };
        if result.spaces {
            let mut best = 0;
            for size in [2, 4, 6, 8, 3, 5, 7] {
                if scores[size] > best {
                    best = scores[size];
                    result.size = size;
                }
            }
            if result.size == 4 && scores[2] > 0 && scores[2] * 3 >= scores[4] * 2 {
                result.size = 2;
            }
        }
        result
    }

    fn whitespace(self, columns: usize) -> String {
        if self.spaces {
            " ".repeat(columns)
        } else {
            "\t".repeat(columns / self.size)
        }
    }
}

fn visible_columns(text: &str, size: usize) -> usize {
    text.graphemes(true).fold(0, |column, cluster| {
        let ch = cluster.chars().next().unwrap_or(' ') as u32;
        if ch == 9 { return column + size - column % size; }
        let wide = matches!(ch, 0x2e80..=0xd7af | 0xf900..=0xfaff | 0xff01..=0xff5e | 0xffe0..=0xffe6
            | 0x1f1e6..=0x1f1ff | 8986 | 8987 | 9200 | 9203 | 9728..=10175 | 11088 | 11093
            | 127744..=128591 | 128640..=128764 | 128992..=129008 | 129280..=129535 | 129648..=129782);
        column + if wide { 2 } else { 1 }
    })
}

pub(super) struct Edit {
    pub range: Range<usize>,
    pub replacement: String,
    pub selection: Range<usize>,
}

pub(super) fn plan(
    text: &str,
    selected: Range<usize>,
    outdent: bool,
    options: Options,
) -> Option<Edit> {
    if selected.start > selected.end
        || selected.end > text.len()
        || !text.is_char_boundary(selected.start)
        || !text.is_char_boundary(selected.end)
    {
        return None;
    }
    let line_start = text[..selected.start].rfind('\n').map_or(0, |p| p + 1);
    let line_end = text[selected.start..]
        .find('\n')
        .map_or(text.len(), |p| selected.start + p);
    let content_end =
        line_end - usize::from(line_end > line_start && text.as_bytes()[line_end - 1] == b'\r');
    let is_block = selected.end > line_end
        || (selected.start == line_start && selected.end >= content_end && !selected.is_empty());
    if !outdent && !is_block {
        let columns = visible_columns(&text[line_start..selected.start], options.size);
        let replacement = if options.spaces {
            " ".repeat(options.size - columns % options.size)
        } else {
            "\t".into()
        };
        let caret = selected.start + replacement.len();
        return Some(Edit {
            range: selected,
            replacement,
            selection: caret..caret,
        });
    }
    let last_position = if selected.end > selected.start && text[..selected.end].ends_with('\n') {
        selected.end - 1
    } else {
        selected.end
    };
    let last_start = text[..last_position].rfind('\n').map_or(0, |p| p + 1);
    let end = text[last_start..]
        .find('\n')
        .map_or(text.len(), |p| last_start + p);
    let mut replacement = String::new();
    let mut changes = Vec::new();
    let mut offset = line_start;
    for line in text[line_start..end].split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        let indent = content
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'\t'))
            .count();
        if (outdent && indent == 0) || (!outdent && content.is_empty() && line_start != last_start)
        {
            replacement.push_str(line);
        } else {
            let columns = visible_columns(&content[..indent], options.size);
            let desired = if outdent {
                columns.saturating_sub(1) / options.size * options.size
            } else {
                columns + options.size - columns % options.size
            };
            let prefix = options.whitespace(desired);
            changes.push((offset, indent, prefix.len()));
            replacement.push_str(&prefix);
            replacement.push_str(&line[indent..]);
        }
        offset += line.len();
    }
    // split_inclusive yields no segment for a single empty line.
    if line_start == end && !outdent {
        replacement = options.whitespace(options.size);
        changes.push((line_start, 0, replacement.len()));
    }
    if changes.is_empty() {
        return None;
    }
    // Monaco's tracked ranges preserve markers in the common part of a replaced
    // prefix. Empty carets stick left only at the line end; selection ends stick left.
    let map = |position: usize, stick_left: bool| {
        let mut delta = 0isize;
        for &(at, old, new) in &changes {
            if position < at || (position == at && (old > 0 || stick_left)) {
                return (position as isize + delta) as usize;
            }
            let common = old.min(new);
            if common > 0
                && (position < at + common
                    || (position == at + common && (old > new || stick_left)))
            {
                return (position as isize + delta) as usize;
            }
            if position < at + old || (position == at + old && stick_left) {
                return (at as isize + delta) as usize + new;
            }
            delta += new as isize - old as isize;
        }
        (position as isize + delta) as usize
    };
    let empty = selected.is_empty();
    let caret_at_end = selected.start == content_end;
    let mut start = map(selected.start, empty && caret_at_end);
    let mut selected_end = map(selected.end, !empty || caret_at_end).max(start);
    if !empty
        && let Some(&(at, old, _)) = changes.first()
        && at == line_start
        && selected.start <= at + old
    {
        start = start.min(selected.start);
    }
    if empty && text[line_start..content_end].trim().is_empty() {
        start = line_start + replacement.trim_end_matches('\r').len();
        selected_end = start;
    }
    let selection = start..selected_end;
    Some(Edit {
        range: line_start..end,
        replacement,
        selection,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_indent_block_edits_match_monaco_reference() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/editor_shift_reference.json"
        ))
        .unwrap();
        for vector in data["vectors"].as_array().unwrap() {
            let text = vector["text"].as_str().unwrap();
            let options = Options {
                size: vector["size"].as_u64().unwrap() as usize,
                spaces: vector["spaces"].as_bool().unwrap(),
            };
            let range = vector["start"].as_u64().unwrap() as usize
                ..vector["end"].as_u64().unwrap() as usize;
            let mut actual = text.to_owned();
            if let Some(edit) = plan(text, range, vector["outdent"].as_bool().unwrap(), options) {
                actual.replace_range(edit.range, &edit.replacement);
                assert!(actual.is_char_boundary(edit.selection.start));
                assert!(actual.is_char_boundary(edit.selection.end));
                assert_eq!(
                    edit.selection.start,
                    vector["selection"][0].as_u64().unwrap() as usize,
                    "selection start {vector}"
                );
                assert_eq!(
                    edit.selection.end,
                    vector["selection"][1].as_u64().unwrap() as usize,
                    "selection end {vector}"
                );
            }
            assert_eq!(actual, vector["expected"].as_str().unwrap(), "{vector}");
        }
    }

    #[test]
    fn native_indent_detection_matches_monaco_reference() {
        let data: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/editor_indent_reference.json"
        ))
        .unwrap();
        for vector in data["vectors"].as_array().unwrap() {
            let text = vector["text"].as_str().unwrap();
            let result = Options::detect(text);
            assert_eq!(
                result.size,
                vector["tabSize"].as_u64().unwrap() as usize,
                "{text:?}"
            );
            assert_eq!(
                result.spaces,
                vector["insertSpaces"].as_bool().unwrap(),
                "{text:?}"
            );
        }
    }

    #[test]
    fn native_indent_edits_preserve_crlf_unicode_and_block_end_boundary() {
        let options = Options {
            size: 4,
            spaces: true,
        };
        let text = "  é😀\r\n  z\r\nuntouched";
        let end = text.find("untouched").unwrap();
        let edit = plan(text, 0..end, false, options).unwrap();
        let mut changed = text.to_owned();
        changed.replace_range(edit.range, &edit.replacement);
        assert_eq!(changed, "    é😀\r\n    z\r\nuntouched");
        assert_eq!(&changed[edit.selection.clone()], "    é😀\r\n    z\r\n");
        let undo_indent = plan(&changed, edit.selection, true, options).unwrap();
        changed.replace_range(undo_indent.range, &undo_indent.replacement);
        assert_eq!(changed, "é😀\r\nz\r\nuntouched");
        let middle = plan("ab😀z", 2..6, false, options).unwrap();
        assert_eq!(middle.replacement, "  ");
        assert_eq!(middle.selection, 4..4);
        assert!(plan("é", 1..2, false, options).is_none());
        let tabs = plan(
            "\tx\n\ty",
            0..5,
            false,
            Options {
                size: 4,
                spaces: false,
            },
        )
        .unwrap();
        assert_eq!(tabs.replacement, "\t\tx\n\t\ty");
    }
}
