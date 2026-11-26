use crate::outline_tree::{Range, Symbol};

#[derive(Clone, Copy)]
struct Position {
    line: usize,
    column: usize,
}

impl Position {
    fn through(self, end: Self) -> Range {
        Range {
            start_line_number: self.line,
            start_column: self.column,
            end_line_number: end.line,
            end_column: end.column,
        }
    }
}

struct Parser<'a> {
    rest: &'a str,
    pos: Position,
    remaining: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.rest.chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.rest = &self.rest[c.len_utf8()..];
        if c == '\r' || c == '\n' {
            if c == '\r' {
                self.rest = self.rest.strip_prefix('\n').unwrap_or(self.rest);
            }
            self.pos.line += 1;
            self.pos.column = 1;
        } else {
            self.pos.column += c.len_utf16();
        }
        Some(c)
    }

    fn trivia(&mut self) {
        loop {
            if self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            } else if self.rest.starts_with("//") {
                while self.peek().is_some_and(|c| c != '\n' && c != '\r') {
                    self.bump();
                }
            } else if self.rest.starts_with("/*") {
                self.bump();
                self.bump();
                while !self.rest.is_empty() && !self.rest.starts_with("*/") {
                    self.bump();
                }
                self.bump();
                self.bump();
            } else {
                break;
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        let start = self.rest;
        if self.bump()? != '"' {
            return None;
        }
        while let Some(c) = self.bump() {
            if c == '"' {
                return serde_json::from_str(&start[..start.len() - self.rest.len()]).ok();
            }
            if c == '\\' {
                self.bump()?;
            }
        }
        None
    }

    fn value(&mut self, depth: usize) -> Option<Symbol> {
        if depth > 128 || self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let start = self.pos;
        let mut children = Vec::new();
        let (kind, detail) = match self.peek()? {
            opening @ ('{' | '[') => {
                let object = opening == '{';
                let close = if object { '}' } else { ']' };
                self.bump();
                loop {
                    self.trivia();
                    match self.peek() {
                        None => break,
                        Some(c) if c == close => {
                            self.bump();
                            break;
                        }
                        Some(',') => {
                            self.bump();
                            continue;
                        }
                        _ => {}
                    }
                    let key_start = self.pos;
                    let property = if object {
                        if self.peek() != Some('"') {
                            break;
                        }
                        let Some(key) = self.string() else {
                            break;
                        };
                        let selection = key_start.through(self.pos);
                        self.trivia();
                        if self.peek() != Some(':') {
                            break;
                        }
                        self.bump();
                        self.trivia();
                        Some((key, selection))
                    } else {
                        None
                    };
                    let Some(mut child) = self.value(depth + 1) else {
                        break;
                    };
                    if let Some((key, selection)) = property {
                        let label = key.replace('\n', "↵");
                        child.name = if label.trim().is_empty() {
                            format!("\"{label}\"")
                        } else {
                            label
                        };
                        child.range.start_line_number = key_start.line;
                        child.range.start_column = key_start.column;
                        child.selection_range = Some(selection);
                    } else {
                        child.name = children.len().to_string();
                        child.detail.clear();
                    }
                    children.push(child);
                }
                let detail = if children.is_empty() {
                    if object { "{}" } else { "[]" }
                } else {
                    ""
                };
                (if object { 1 } else { 17 }, detail.to_owned())
            }
            '"' => (14, self.string()?),
            _ => {
                let initial = self.rest;
                while self
                    .peek()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-'))
                {
                    self.bump();
                }
                let token = &initial[..initial.len() - self.rest.len()];
                match token {
                    "true" | "false" => (16, token.to_owned()),
                    "null" => (12, token.to_owned()),
                    _ => (15, serde_json::from_str::<f64>(token).ok()?.to_string()),
                }
            }
        };
        let range = start.through(self.pos);
        Some(Symbol {
            name: String::new(),
            kind,
            detail,
            range,
            selection_range: Some(range),
            deprecated: false,
            key: String::new(),
            children,
        })
    }
}

/// Reads ordered properties (including duplicates) and array elements from an editor buffer.
pub fn document_symbols(text: &str) -> Vec<Symbol> {
    let mut parser = Parser {
        rest: text,
        pos: Position { line: 1, column: 1 },
        remaining: 50000,
    };
    parser.trivia();
    parser.value(0).map(|s| s.children).unwrap_or_default()
}

/// Convert the language provider's UTF-16 column to the editor's character offset.
pub fn char_offset(text: &str, line: usize, column: usize) -> usize {
    let mut pos = Position { line: 1, column: 1 };
    let mut chars = text.chars().peekable();
    let mut offset = 0;
    while (pos.line, pos.column) < (line, column) {
        let Some(c) = chars.next() else {
            break;
        };
        offset += 1;
        if c == '\r' || c == '\n' {
            if c == '\r' && chars.next_if_eq(&'\n').is_some() {
                offset += 1;
            }
            pos.line += 1;
            pos.column = 1;
        } else {
            pos.column += c.len_utf16();
        }
    }
    offset
}
