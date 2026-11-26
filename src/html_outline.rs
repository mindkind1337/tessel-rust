//! HTML document symbols for the Files outline. Keep source spelling and ranges
//! from the editor buffer, including unclosed elements and duplicate siblings.
use crate::outline_tree::{Range, Symbol};

#[derive(Clone, Copy)]
struct Position {
    line: usize,
    column: usize,
}

struct Source<'a> {
    rest: &'a str,
    pos: Position,
}
impl Source<'_> {
    fn consume(&mut self, count: usize) {
        let (part, rest) = self.rest.split_at(count);
        let mut chars = part.chars().peekable();
        while let Some(c) = chars.next() {
            if matches!(c, '\n' | '\r') {
                if c == '\r' {
                    chars.next_if_eq(&'\n');
                }
                self.pos.line += 1;
                self.pos.column = 1;
            } else {
                self.pos.column += c.len_utf16();
            }
        }
        self.rest = rest;
    }
}

struct Element<'a> {
    tag: &'a str,
    symbol: Symbol,
}
fn finish(stack: &mut Vec<Element<'_>>, roots: &mut Vec<Symbol>, pos: Position) {
    let mut symbol = stack.pop().unwrap().symbol;
    symbol.range.end_line_number = pos.line;
    symbol.range.end_column = pos.column;
    symbol.selection_range = Some(symbol.range);
    if let Some(parent) = stack.last_mut() {
        parent.symbol.children.push(symbol);
    } else {
        roots.push(symbol);
    }
}

fn tag_name(s: &str) -> &str {
    let len = s
        .bytes()
        .enumerate()
        .take_while(|&(i, c)| {
            c.is_ascii_alphanumeric()
                || matches!(c, b'_' | b':')
                || (i > 0 && matches!(c, b'-' | b'.'))
        })
        .count();
    &s[..len]
}

#[derive(Default)]
struct Attributes<'a> {
    id: Option<&'a str>,
    classes: Option<&'a str>,
    script_type: Option<&'a str>,
}
fn attribute_value(s: &str) -> (&str, &str) {
    if let Some(quote @ ('\'' | '"')) = s.chars().next() {
        let end = s[1..].find(quote).map_or(s.len(), |i| i + 2);
        return s.split_at(end);
    }
    let mut end = s
        .find(|c: char| c.is_whitespace() || matches!(c, '>' | '<' | '=' | '\'' | '"' | '`'))
        .unwrap_or(s.len());
    if end > 0 && s[end..].starts_with('>') && s[..end].ends_with('/') {
        end -= 1;
    }
    s.split_at(end)
}

fn attributes(mut s: &str) -> (Attributes<'_>, &str, bool) {
    let mut result = Attributes::default();
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix("/>") {
            return (result, rest, true);
        }
        if let Some(rest) = s.strip_prefix('>') {
            return (result, rest, false);
        }
        if s.is_empty() || s.starts_with('<') {
            return (result, s, false);
        }
        let end = s
            .find(|c: char| c.is_whitespace() || matches!(c, '/' | '>' | '<' | '=' | '\'' | '"'))
            .unwrap_or(s.len());
        if end == 0 {
            s = &s[1..];
            continue;
        }
        let key = &s[..end];
        s = s[end..].trim_start();
        let mut value = None;
        if let Some(rest) = s.strip_prefix('=') {
            let (v, remaining) = attribute_value(rest.trim_start());
            value = (!v.is_empty()).then_some(v);
            s = remaining;
        }
        match key {
            "id" => result.id = value,
            "class" => result.classes = value,
            k if k.eq_ignore_ascii_case("type") => result.script_type = value,
            _ => {}
        }
    }
}

fn element_name(tag: &str, attrs: &Attributes<'_>) -> String {
    let mut name = tag.to_owned();
    if let Some(id) = attrs.id {
        name.push('#');
        name.extend(id.chars().filter(|c| !matches!(c, '\'' | '"')));
    }
    if let Some(classes) = attrs.classes {
        name.push('.');
        let mut whitespace = false;
        for c in classes.chars().filter(|c| !matches!(c, '\'' | '"')) {
            if c.is_whitespace() {
                if !whitespace {
                    name.push('.');
                }
                whitespace = true;
            } else {
                name.push(c);
                whitespace = false;
            }
        }
    }
    name
}

fn raw_content_len(input: &str, script: bool) -> usize {
    use std::sync::LazyLock;
    static SCRIPT: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)<!--|-->|</?script\s*/?>?").unwrap());
    static STYLE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)</style").unwrap());
    if !script {
        return STYLE.find(input).map_or(input.len(), |m| m.start());
    }
    // HTML script data can contain a nested script example in an escaped comment.
    let mut escaped = false;
    let mut double_escaped = false;
    for token in SCRIPT.find_iter(input) {
        match token.as_str() {
            "<!--" => escaped = true,
            "-->" => {
                escaped = false;
                double_escaped = false;
            }
            close if close.starts_with("</") => {
                if !double_escaped {
                    return token.start();
                }
                double_escaped = false;
            }
            _ if escaped => double_escaped = true,
            _ => {}
        }
    }
    input.len()
}

fn void(tag: &str) -> bool {
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

pub fn document_symbols(text: &str) -> Vec<Symbol> {
    let mut source = Source {
        rest: text,
        pos: Position { line: 1, column: 1 },
    };
    let mut roots = Vec::new();
    let mut stack: Vec<Element<'_>> = Vec::new();
    let mut count = 0;
    while let Some(offset) = source.rest.find('<') {
        source.consume(offset);
        let start = source.pos;
        let input = source.rest;
        let closing = input.starts_with("</");
        let content = input[if closing { 2 } else { 1 }..].trim_start();
        let tag = tag_name(content);
        if let Some(comment) = input.strip_prefix("<!--") {
            source.consume(comment.find("-->").map_or(input.len(), |i| i + 7));
            continue;
        }
        if input
            .get(..9)
            .is_some_and(|s| s.eq_ignore_ascii_case("<!doctype"))
        {
            source.consume(input.find('>').map_or(input.len(), |i| i + 1));
            continue;
        }
        let (attrs, rest, self_closing) = attributes(&content[tag.len()..]);
        source.consume(input.len() - rest.len());
        if closing {
            if let Some(index) = stack
                .iter()
                .rposition(|el| el.tag.eq_ignore_ascii_case(tag))
            {
                while stack.len() > index + 1 {
                    finish(&mut stack, &mut roots, start);
                }
                finish(&mut stack, &mut roots, source.pos);
            }
        } else {
            if stack.len() >= 128 || count >= 50000 {
                break;
            }
            count += 1;
            let range = Range {
                start_line_number: start.line,
                start_column: start.column,
                ..Default::default()
            };
            let script_type = attrs.script_type.unwrap_or("").trim_matches(['\'', '"']);
            let raw = tag.eq_ignore_ascii_case("style")
                || (tag.eq_ignore_ascii_case("script")
                    && !matches!(script_type, "text/html" | "text/x-handlebars-template"));
            stack.push(Element {
                tag,
                symbol: Symbol {
                    name: element_name(if tag.is_empty() { "?" } else { tag }, &attrs),
                    kind: 7,
                    range,
                    selection_range: None,
                    detail: String::new(),
                    deprecated: false,
                    key: String::new(),
                    children: Vec::new(),
                },
            });
            if self_closing
                || rest.starts_with('<') && !input[..input.len() - rest.len()].ends_with('>')
                || void(tag)
            {
                finish(&mut stack, &mut roots, source.pos);
            } else if raw {
                source.consume(raw_content_len(
                    source.rest,
                    tag.eq_ignore_ascii_case("script"),
                ));
            }
        }
    }
    source.consume(source.rest.len());
    while !stack.is_empty() {
        finish(&mut stack, &mut roots, source.pos);
    }
    roots
}
