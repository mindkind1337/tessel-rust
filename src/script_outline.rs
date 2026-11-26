//! Document symbols for JavaScript and TypeScript source. Parser spans use UTF-8
//! bytes, while the editor addresses a line and column in UTF-16 code units.
use crate::outline_tree::{Range, Symbol};
use oxc_allocator::Allocator;
use oxc_ast::{
    AstKind,
    ast::{BindingPattern, ClassType, FunctionType, MethodDefinitionKind, PropertyKey},
};
use oxc_ast_visit::Visit;
use oxc_parser::{Parser, ParserReturn, config::TokensParserConfig};
use oxc_span::{GetSpan, SourceType, Span};

struct Positions<'s> {
    text: &'s str,
    checkpoints: Vec<(usize, usize, usize)>,
}
impl<'s> Positions<'s> {
    fn new(text: &'s str) -> Self {
        let mut points = vec![(0, 1, 1)];
        let (mut line, mut column) = (1, 1);
        let mut chars = text.char_indices().peekable();
        while let Some((offset, c)) = chars.next() {
            if matches!(c, '\r' | '\n') {
                let mut next = offset + 1;
                if c == '\r' && chars.next_if(|(_, c)| *c == '\n').is_some() {
                    next += 1;
                }
                line += 1;
                column = 1;
                points.push((next, line, column));
            } else {
                column += c.len_utf16();
                let next = offset + c.len_utf8();
                if next - points.last().unwrap().0 >= 128 {
                    points.push((next, line, column));
                }
            }
        }
        Self {
            text,
            checkpoints: points,
        }
    }
    fn position(&self, byte: u32) -> (usize, usize) {
        let byte = (byte as usize).min(self.text.len());
        let i = self.checkpoints.partition_point(|p| p.0 <= byte) - 1;
        let (start, line, column) = self.checkpoints[i];
        (line, column + self.text[start..byte].encode_utf16().count())
    }
    fn range(&self, span: Span) -> Range {
        let (start_line_number, start_column) = self.position(span.start);
        let (end_line_number, end_column) = self.position(span.end);
        Range {
            start_line_number,
            start_column,
            end_line_number,
            end_column,
        }
    }
}

struct Outline<'a> {
    positions: Positions<'a>,
    parsed_source: &'a str,
    ancestors: Vec<(AstKind<'a>, bool)>,
    open: Vec<Symbol>,
    roots: Vec<Symbol>,
    count: usize,
}
impl Outline<'_> {
    fn key_span(key: &PropertyKey<'_>) -> Option<Span> {
        (!key.is_computed()).then(|| key.span())
    }
    fn entry(&self, node: AstKind<'_>) -> Option<(String, u8, Span)> {
        use AstKind::*;
        let source = self.parsed_source;
        let parent = self.ancestors.last().map(|&(p, _)| p);
        let mut span = node.span();
        let default = matches!(parent, Some(ExportDefaultDeclaration(_)));
        let (name, kind) = match node {
            Function(f)
                if matches!(
                    f.r#type,
                    FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction
                ) =>
            {
                (
                    f.id.as_ref()
                        .map(|i| i.name.as_str())
                        .or(default.then_some("default"))?,
                    11,
                )
            }
            Class(c) if c.r#type == ClassType::ClassDeclaration => (
                c.id.as_ref()
                    .map(|i| i.name.as_str())
                    .or(default.then_some("default"))?,
                4,
            ),
            VariableDeclarator(v) => match &v.id {
                BindingPattern::BindingIdentifier(id) => (id.name.as_str(), 12),
                _ => return None,
            },
            BindingIdentifier(id) => {
                let mut in_pattern = false;
                let mut have_element_span = false;
                for &(ancestor, _) in self.ancestors.iter().rev() {
                    match ancestor {
                        ObjectPattern(_) | ArrayPattern(_) => {
                            in_pattern = true;
                            have_element_span = true;
                        }
                        BindingProperty(_) | BindingRestElement(_) | AssignmentPattern(_) => {
                            if !have_element_span {
                                span = ancestor.span();
                            }
                        }
                        VariableDeclarator(_) if in_pattern => break,
                        _ => return None,
                    }
                }
                if !in_pattern {
                    return None;
                }
                (id.name.as_str(), 12)
            }
            FormalParameter(p) if p.accessibility.is_some() || p.readonly || p.r#override => {
                (p.pattern.get_identifier_name()?.as_str(), 6)
            }
            MethodDefinition(m) => (
                Self::key_span(&m.key)?.source_text(source),
                match m.kind {
                    MethodDefinitionKind::Constructor => 12,
                    MethodDefinitionKind::Method => 5,
                    _ => 6,
                },
            ),
            PropertyDefinition(p) => (Self::key_span(&p.key)?.source_text(source), 6),
            ObjectProperty(p) => (
                Self::key_span(&p.key)?.source_text(source),
                if p.kind == oxc_ast::ast::PropertyKind::Init
                    && (p.method
                        || matches!(
                            p.value,
                            oxc_ast::ast::Expression::ArrowFunctionExpression(_)
                                | oxc_ast::ast::Expression::FunctionExpression(_)
                        ))
                {
                    5
                } else {
                    6
                },
            ),
            TSInterfaceDeclaration(d) => (d.id.name.as_str(), 10),
            TSTypeAliasDeclaration(d) => (d.id.name.as_str(), 12),
            TSNamespaceDeclaration(d) => (d.id.name.as_str(), 1),
            TSExternalModuleDeclaration(d) => (d.id.span.source_text(source), 1),
            TSMethodSignature(m) => (Self::key_span(&m.key)?.source_text(source), 5),
            TSPropertySignature(p) => (Self::key_span(&p.key)?.source_text(source), 6),
            TSEnumDeclaration(d) => (d.id.name.as_str(), 9),
            TSEnumMember(m) => (m.id.span().source_text(source), 12),
            ImportSpecifier(i) => (i.local.name.as_str(), 12),
            ImportDefaultSpecifier(i) => (i.local.name.as_str(), 12),
            ImportNamespaceSpecifier(i) => (i.local.name.as_str(), 12),
            _ => return None,
        };
        if let Some(p @ (ExportDefaultDeclaration(_) | ExportDeclaration(_))) = parent {
            span.start = p.span().start;
        }
        Some((name.to_owned(), kind, span))
    }
}
// The TypeScript navigation tree folds function overloads into one entry. Monaco
// uses the first declaration's span; declarations in later bodies still contribute children.
fn finish_level(symbols: &mut Vec<Symbol>) {
    symbols.sort_by(|a, b| {
        crate::outline_tree::compare_names(
            a.name.trim_matches(['\'', '"']),
            b.name.trim_matches(['\'', '"']),
        )
    });
    symbols.dedup_by(|later, first| {
        if first.kind == 11 && later.kind == 11 && first.name == later.name {
            first.children.append(&mut later.children);
            finish_level(&mut first.children);
            true
        } else {
            false
        }
    });
}
impl<'a> Visit<'a> for Outline<'a> {
    // The TypeScript navigation tree treats a type alias as a single item.
    fn visit_ts_type_alias_declaration(
        &mut self,
        declaration: &oxc_ast::ast::TSTypeAliasDeclaration<'a>,
    ) {
        let node = AstKind::TSTypeAliasDeclaration(self.alloc(declaration));
        self.enter_node(node);
        self.leave_node(node);
    }
    fn enter_node(&mut self, node: AstKind<'a>) {
        let entry = if self.count < 50_000
            && self.open.len() < 128
            && (node.span().start as usize) < self.positions.text.len()
        {
            self.entry(node)
        } else {
            None
        };
        let opened = entry.is_some();
        if let Some((name, kind, span)) = entry {
            let range = self.positions.range(span);
            self.open.push(Symbol {
                name,
                kind,
                range,
                selection_range: Some(range),
                children: Vec::new(),
                detail: String::new(),
                key: String::new(),
                deprecated: false,
            });
            self.count += 1;
        }
        self.ancestors.push((node, opened));
    }
    fn leave_node(&mut self, node: AstKind<'a>) {
        if self.ancestors.pop().unwrap().1 {
            let mut symbol = self.open.pop().unwrap();
            finish_level(&mut symbol.children);
            // Parameter properties belong to the class; declarations in the body belong to the constructor.
            let parent_depth = if matches!(node, AstKind::FormalParameter(_))
                && self.open.last().is_some_and(|s| s.name == "constructor")
            {
                2
            } else {
                1
            };
            if let Some(parent) = self.open.iter_mut().rev().nth(parent_depth - 1) {
                parent.children.push(symbol);
            } else {
                self.roots.push(symbol);
            }
        }
    }
}

pub fn document_symbols(source: &str, typescript: bool) -> Vec<Symbol> {
    let allocator = Allocator::default();
    let language = if typescript {
        SourceType::ts()
    } else {
        SourceType::mjs()
    };
    let parsed = Parser::new(&allocator, source, language).parse();
    if !parsed.fatal_error {
        return symbols_from_program(&parsed, source, source);
    }
    // Oxc discards its tree on a fatal error. When the missing token is at EOF,
    // complete a private copy and project every range back onto the draft.
    let Some(mut token) = eof_completion(&parsed, source.len()) else {
        return Vec::new();
    };
    let mut draft = source.to_owned();
    draft.push('\n'); // a trailing line comment must end before the completion
    for _ in 0..12 {
        draft.push_str(token);
        let arena = Allocator::default();
        let result = Parser::new(&arena, &draft, language)
            .with_config(TokensParserConfig)
            .parse();
        if !result.fatal_error {
            let end = result
                .tokens
                .iter()
                .rev()
                .find(|t| t.end() as usize <= source.len() && t.end() > t.start())
                .map_or(0, |t| t.end() as usize);
            return symbols_from_program(&result, &draft, &source[..end]);
        }
        let Some(next) = eof_completion(&result, draft.len()) else {
            break;
        };
        token = next;
    }
    Vec::new()
}

// These diagnostics come from the pinned Oxc parser. Only insert punctuation
// (or an anonymous expression operand) at the reported end of input.
fn eof_completion(parsed: &ParserReturn<'_>, length: usize) -> Option<&'static str> {
    let error = parsed.diagnostics.iter().rev().find(|e| {
        e.labels.iter().any(|l| {
            (l.is_empty() || e.message == "Expected function body")
                && l.span().end as usize == length
        })
    })?;
    match error.message.as_ref() {
        "Expected `}` but found `EOF`" | "Expected `,` or `}` but found `EOF`" => Some("}"),
        "Expected `)` but found `EOF`" | "Expected `,` or `)` but found `EOF`" => Some(")"),
        "Expected `]` but found `EOF`" | "Expected `,` or `]` but found `EOF`" => Some("]"),
        "Expected function body" | "Expected `{` but found `EOF`" => Some("{}"),
        "Unexpected token" => Some("0"),
        _ => None,
    }
}

fn symbols_from_program(
    parsed: &ParserReturn<'_>,
    parsed_source: &str,
    source: &str,
) -> Vec<Symbol> {
    let mut outline = Outline {
        positions: Positions::new(source),
        parsed_source,
        ancestors: Vec::new(),
        open: Vec::new(),
        roots: Vec::new(),
        count: 0,
    };
    outline.visit_program(&parsed.program);
    finish_level(&mut outline.roots);
    outline.roots
}
