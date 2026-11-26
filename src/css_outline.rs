//! Native CSS document symbols. CSS Syntax tokenization keeps strings, escaped
//! delimiters and functional selectors intact while the outline walks rule bodies.
use crate::outline_tree::{Range, Symbol};
use cssparser::{ParseError, Parser, Token};

#[derive(Clone, Copy)]
struct Point {
    byte: usize,
    line: usize,
    column: usize,
}
impl Point {
    fn at(parser: &Parser<'_>) -> Self {
        let pos = parser.current_source_location();
        Self {
            byte: parser.position().byte_index(),
            line: pos.line as usize + 1,
            column: pos.column as usize,
        }
    }
    fn until(self, end: Self) -> Range {
        Range {
            start_line_number: self.line,
            start_column: self.column,
            end_line_number: end.line,
            end_column: end.column,
        }
    }
}

#[derive(Clone, Copy)]
struct Span {
    start: Point,
    end: Point,
}
impl Span {
    fn text(self, source: &str) -> &str {
        &source[self.start.byte..self.end.byte]
    }
}

#[derive(Default)]
struct Prelude {
    first: Option<Span>,
    second: Option<Span>,
    current: Option<Span>,
    selectors: Vec<Span>,
}
impl Prelude {
    fn token(&mut self, span: Span) {
        if self.first.is_none() {
            self.first = Some(span);
        } else if self.second.is_none() {
            self.second = Some(span);
        }
        self.current.get_or_insert(span).end = span.end;
    }
    fn comma(&mut self) {
        if let Some(selector) = self.current.take()
            && self.selectors.len() < MAX_SYMBOLS
        {
            self.selectors.push(selector);
        }
    }
}

const MAX_SYMBOLS: usize = 50_000;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    Rules,
    Declarations,
    Keyframes,
}

fn symbol(name: String, kind: u8, start: Point, selected: Span) -> Symbol {
    Symbol {
        name,
        kind,
        range: start.until(start),
        selection_range: Some(selected.start.until(selected.end)),
        detail: String::new(),
        deprecated: false,
        key: String::new(),
        children: Vec::new(),
    }
}

struct Rule {
    symbols: Vec<Symbol>,
    body: Option<Context>,
}
fn rule(
    header: &mut Prelude,
    source: &str,
    context: Context,
    block: bool,
    budget: &mut usize,
) -> Rule {
    let mut result = Rule {
        symbols: Vec::new(),
        body: None,
    };
    let Some(first) = header.first else {
        return result;
    };
    let keyword = first.text(source);
    if keyword.starts_with('@') {
        let empty = Span {
            start: first.start,
            end: first.start,
        };
        let item = match keyword.to_ascii_lowercase().as_str() {
            "@media" => {
                result.body = Some(Context::Rules);
                let list = header
                    .second
                    .map(|s| Span {
                        start: s.start,
                        end: header.current.map_or(s.end, |v| v.end),
                    })
                    .unwrap_or(Span {
                        start: first.end,
                        end: first.end,
                    });
                Some(symbol(
                    format!("@media {}", list.text(source)),
                    1,
                    first.start,
                    list,
                ))
            }
            "@font-face" => {
                result.body = Some(Context::Declarations);
                Some(symbol("@font-face".into(), 4, first.start, empty))
            }
            "@keyframes" | "@-webkit-keyframes" | "@-moz-keyframes" | "@-o-keyframes" => {
                result.body = Some(Context::Keyframes);
                let name = header
                    .second
                    .filter(|v| !v.text(source).starts_with(['\'', '"']))
                    .unwrap_or(empty);
                Some(symbol(
                    format!("@keyframes {}", name.text(source)),
                    4,
                    first.start,
                    name,
                ))
            }
            "@supports" | "@container" | "@layer" => {
                result.body = Some(context);
                None
            }
            _ => None,
        };
        if let Some(item) = item
            && *budget > 0
        {
            result.symbols.push(item);
            *budget -= 1;
        }
    } else if block || context == Context::Rules {
        result.body = Some(Context::Declarations);
        if context != Context::Keyframes {
            header.comma();
            for selector in header.selectors.drain(..).take(*budget) {
                result.symbols.push(symbol(
                    selector.text(source).into(),
                    4,
                    selector.start,
                    selector,
                ));
                *budget -= 1;
            }
        }
    }
    result
}

fn skip_block(parser: &mut Parser<'_>) {
    let _: Result<(), ParseError<()>> = parser.parse_nested_block(|nested| {
        while nested.next_including_whitespace_and_comments().is_ok() {}
        Ok(())
    });
}

fn finish(mut rule: Rule, end: Point, children: Vec<Symbol>, output: &mut Vec<Symbol>) {
    for node in &mut rule.symbols {
        node.range.end_line_number = end.line;
        node.range.end_column = end.column;
    }
    if let Some(last) = rule.symbols.last_mut() {
        last.children = children;
        output.append(&mut rule.symbols);
    } else {
        output.extend(children);
    }
}

fn walk(
    parser: &mut Parser<'_>,
    source: &str,
    context: Context,
    depth: usize,
    budget: &mut usize,
) -> (Vec<Symbol>, Point) {
    let mut output = Vec::new();
    let mut header = Prelude::default();
    let mut last = Point::at(parser);
    loop {
        let start = Point::at(parser);
        let Ok(token) = parser.next_including_whitespace_and_comments() else {
            break;
        };
        match token {
            Token::WhiteSpace(_) | Token::Comment(_) => continue,
            Token::Semicolon => {
                let item = rule(&mut header, source, context, false, budget);
                finish(item, Point::at(parser), Vec::new(), &mut output);
                header = Prelude::default();
            }
            Token::CurlyBracketBlock => {
                let item = rule(&mut header, source, context, true, budget);
                let mut children = Vec::new();
                let mut end = Point::at(parser);
                if let Some(child_context) = item.body
                    && depth < 128
                    && *budget > 0
                {
                    let mut before_close = end;
                    let _: Result<(), ParseError<()>> = parser.parse_nested_block(|nested| {
                        (children, end) = walk(nested, source, child_context, depth + 1, budget);
                        before_close = Point::at(nested);
                        Ok(())
                    });
                    if parser.position().byte_index() > before_close.byte {
                        end = Point::at(parser);
                    }
                } else {
                    skip_block(parser);
                    end = Point::at(parser);
                }
                finish(item, end, children, &mut output);
                header = Prelude::default();
                last = end;
                continue;
            }
            Token::Comma => header.comma(),
            _ => {
                if matches!(
                    token,
                    Token::Function(_) | Token::SquareBracketBlock | Token::ParenthesisBlock
                ) {
                    skip_block(parser);
                }
                header.token(Span {
                    start,
                    end: Point::at(parser),
                });
            }
        }
        last = Point::at(parser);
    }
    let item = rule(&mut header, source, context, false, budget);
    finish(item, last, Vec::new(), &mut output);
    (output, last)
}

pub fn document_symbols(source: &str) -> Vec<Symbol> {
    let mut budget = MAX_SYMBOLS;
    walk(
        &mut Parser::new(source),
        source,
        Context::Rules,
        0,
        &mut budget,
    )
    .0
}
