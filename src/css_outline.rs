//! Native CSS document symbols. CSS Syntax tokenization keeps strings, escaped
//! delimiters and functional selectors intact while the outline walks rule bodies.
use crate::outline_tree::{Range, Symbol};
use cssparser::{ParseError, Parser, Token};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Css,
    Scss,
    Less,
}

/// CSS Syntax already handles strings, block comments and url() tokens. Mask
/// preprocessor line comments without moving the original source byte offsets.
fn parser_source(source: &str, language: Language) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if language == Language::Css || !source.contains("//") {
        return Cow::Borrowed(source);
    }
    let bytes = source.as_bytes();
    let mut masked = None;
    let mut i = 0;
    let mut quote = None;
    let mut url_depth = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'\\' {
            i += 2;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
        } else if matches!(c, b'\'' | b'"') {
            quote = Some(c);
        } else if bytes[i..].starts_with(b"/*") {
            i = bytes[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(bytes.len(), |n| i + n + 4);
            continue;
        } else if c == b'('
            && (url_depth > 0 || (i >= 3 && bytes[i - 3..i].eq_ignore_ascii_case(b"url")))
        {
            url_depth += 1;
        } else if c == b')' {
            url_depth = url_depth.saturating_sub(1);
        } else if url_depth == 0 && bytes[i..].starts_with(b"//") {
            let end = bytes[i..]
                .iter()
                .position(|c| matches!(c, b'\r' | b'\n'))
                .map_or(bytes.len(), |n| i + n);
            masked.get_or_insert_with(|| bytes.to_vec())[i..end].fill(b' ');
            i = end;
            continue;
        }
        i += 1;
    }
    masked.map_or(Cow::Borrowed(source), |bytes| {
        Cow::Owned(String::from_utf8(bytes).expect("whole comment replaced by ASCII"))
    })
}

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
    third: Option<Span>,
    current: Option<Span>,
    selectors: Vec<Span>,
}
impl Prelude {
    fn token(&mut self, span: Span) {
        if self.first.is_none() {
            self.first = Some(span);
        } else if self.second.is_none() {
            self.second = Some(span);
        } else if self.third.is_none() {
            self.third = Some(span);
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
    Properties,
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
fn identifier(span: Span, source: &str) -> Span {
    let raw = span.text(source);
    let name = raw.split_once('(').map_or(raw, |(name, _)| name);
    Span {
        start: span.start,
        end: Point {
            byte: span.start.byte + name.len(),
            line: span.start.line,
            column: span.start.column + name.encode_utf16().count(),
        },
    }
}

fn dialect_rule(
    header: &Prelude,
    source: &str,
    language: Language,
    block: bool,
    context: Context,
) -> Option<Rule> {
    let first = header.first?;
    let second = header.second?;
    let keyword = first.text(source);
    let body = block.then_some(Context::Declarations);
    let (kind, name) = match language {
        Language::Scss if keyword == "$" && header.third.is_some_and(|s| s.text(source) == ":") => {
            (
                12,
                Span {
                    start: first.start,
                    end: second.end,
                },
            )
        }
        Language::Less if keyword.starts_with('@') && second.text(source) == ":" => (12, first),
        Language::Scss if matches!(keyword, "@mixin" | "@function") => (
            if keyword == "@mixin" { 5 } else { 11 },
            identifier(second, source),
        ),
        Language::Less if keyword == "." || keyword.starts_with('#') => {
            let name = if keyword == "." && second.text(source).contains('(') {
                Span {
                    start: first.start,
                    end: identifier(second, source).end,
                }
            } else if keyword == "."
                && header
                    .third
                    .is_some_and(|s| s.text(source).starts_with('('))
            {
                Span {
                    start: first.start,
                    end: second.end,
                }
            } else if keyword.starts_with('#') && second.text(source).starts_with('(') {
                first
            } else if !block {
                return Some(Rule {
                    symbols: Vec::new(),
                    body: None,
                });
            } else {
                return None;
            };
            if !block {
                return Some(Rule {
                    symbols: Vec::new(),
                    body: None,
                });
            }
            (5, name)
        }
        Language::Scss
            if context == Context::Declarations
                && second.text(source) == ":"
                && header.third.is_none_or(|v| v.start.byte > second.end.byte)
                && block =>
        {
            return Some(Rule {
                symbols: Vec::new(),
                body: Some(Context::Properties),
            });
        }
        _ => return None,
    };
    Some(Rule {
        symbols: vec![symbol(name.text(source).into(), kind, first.start, name)],
        body,
    })
}

fn rule(
    language: Language,
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
    if *budget == 0 {
        return result;
    }
    if language != Language::Css
        && let Some(special) = dialect_rule(header, source, language, block, context)
    {
        *budget -= special.symbols.len();
        return special;
    }
    let keyword = first.text(source);
    if keyword.starts_with('@') && !(language == Language::Scss && keyword == "@at-root") {
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
            "@if" | "@else" | "@each" | "@for" | "@while" | "@include"
                if language == Language::Scss =>
            {
                result.body = Some(context);
                None
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
        if !matches!(context, Context::Keyframes | Context::Properties) {
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
    if let Some(last) = rule.symbols.last_mut()
        && last.kind != 12
    {
        last.children = children;
        output.append(&mut rule.symbols);
    } else {
        output.append(&mut rule.symbols);
        output.extend(children);
    }
}

fn walk(
    language: Language,
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
                let item = rule(language, &mut header, source, context, false, budget);
                let end = if item.symbols.first().is_some_and(|s| s.kind == 12) {
                    last
                } else {
                    Point::at(parser)
                };
                finish(item, end, Vec::new(), &mut output);
                header = Prelude::default();
            }
            Token::CurlyBracketBlock
                if start.byte > 0
                    && match language {
                        Language::Scss => source.as_bytes()[start.byte - 1] == b'#',
                        Language::Less => source.as_bytes()[start.byte - 1] == b'@',
                        Language::Css => false,
                    } =>
            {
                skip_block(parser);
                header.token(Span {
                    start,
                    end: Point::at(parser),
                });
            }
            Token::CurlyBracketBlock => {
                let item = rule(language, &mut header, source, context, true, budget);
                let mut children = Vec::new();
                let mut end = Point::at(parser);
                if let Some(child_context) = item.body
                    && depth < 128
                    && *budget > 0
                {
                    let mut before_close = end;
                    let _: Result<(), ParseError<()>> = parser.parse_nested_block(|nested| {
                        (children, end) =
                            walk(language, nested, source, child_context, depth + 1, budget);
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
    let item = rule(language, &mut header, source, context, false, budget);
    finish(item, last, Vec::new(), &mut output);
    (output, last)
}

pub fn document_symbols(source: &str, language: Language) -> Vec<Symbol> {
    let mut budget = MAX_SYMBOLS;
    let input = parser_source(source, language);
    walk(
        language,
        &mut Parser::new(&input),
        source,
        Context::Rules,
        0,
        &mut budget,
    )
    .0
}
