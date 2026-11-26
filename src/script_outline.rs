//! Document symbols for JavaScript and TypeScript source. Parser spans use UTF-8
//! bytes, while the editor addresses a line and column in UTF-16 code units.
use crate::outline_tree::{Range, Symbol};
use oxc_allocator::Allocator;
use oxc_ast::{
    AstKind,
    ast::{
        Argument, BindingPattern, CallExpression, ClassType, Expression, FunctionType,
        MethodDefinitionKind, PropertyKey,
    },
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

// The navigation service displays at most 150 UTF-16 units of each call label.
fn label_text(raw: &str) -> String {
    let units: Vec<_> = raw.encode_utf16().take(151).collect();
    let mut text = String::from_utf16_lossy(&units[..units.len().min(150)]);
    if units.len() > 150 {
        text.push_str("...");
    }
    let newline = |c: char| matches!(c, '\r' | '\n' | '\u{2028}' | '\u{2029}');
    let mut chars = text.chars().peekable();
    let mut label = String::new();
    while let Some(c) = chars.next() {
        if !(newline(c) || c == '\\' && chars.peek().copied().is_some_and(newline)) {
            label.push(c);
        }
    }
    label
}

fn callback_name(call: &CallExpression<'_>, source: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut base = &call.callee;
    while let Expression::StaticMemberExpression(member) = base {
        parts.push(member.property.name.as_str());
        base = &member.object;
    }
    if let Expression::Identifier(id) = base {
        parts.push(id.name.as_str());
    }
    if parts.is_empty() {
        return None;
    }
    parts.reverse();
    let name = label_text(&parts.join("."));
    if name.encode_utf16().count() > 150 {
        return Some(format!("{name} callback"));
    }
    let args = call
        .arguments
        .iter()
        .filter(|a| matches!(a, Argument::StringLiteral(_) | Argument::TemplateLiteral(_)))
        .map(|a| a.span().source_text(source))
        .collect::<Vec<_>>();
    Some(format!("{name}({}) callback", label_text(&args.join(", "))))
}

// TypeScript sorts equal names by syntax kind and gives spreads an unnamed sort key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NavigationKind {
    Constructor,
    Expression,
    Arrow,
    Other,
    Declaration,
    Property,
    Shorthand,
    ExportSpecifier,
    Spread,
}
type NavigationKinds = std::collections::BTreeMap<Range, NavigationKind>;
struct Outline<'a> {
    positions: Positions<'a>,
    parsed_source: &'a str,
    ancestors: Vec<(AstKind<'a>, bool)>,
    open: Vec<Symbol>,
    roots: Vec<Symbol>,
    count: usize,
    kinds: NavigationKinds,
    spread_names: std::collections::HashSet<(u32, &'a str)>,
    call_names: std::collections::HashMap<(u32, u32), Option<String>>,
}
impl Outline<'_> {
    fn key_span(key: &PropertyKey<'_>) -> Option<Span> {
        (!key.is_computed()).then(|| key.span())
    }
    fn entry(&mut self, node: AstKind<'_>) -> Option<(String, u8, Span)> {
        use AstKind::*;
        let source = self.parsed_source;
        let parent = self.ancestors.last().map(|&(p, _)| p);
        let mut span = node.span();
        let default = matches!(parent, Some(ExportDefaultDeclaration(_)));
        // A direct initializer is already represented by its variable or member.
        let expression = match node {
            ArrowFunctionExpression(_) => Some((None, 11)),
            Function(f) if f.r#type == FunctionType::FunctionExpression => {
                Some((f.id.as_ref().map(|id| id.name.as_str()), 11))
            }
            Class(c) if c.r#type == ClassType::ClassExpression => {
                Some((c.id.as_ref().map(|id| id.name.as_str()), 4))
            }
            _ => None,
        };
        if let Some((id, kind)) = expression {
            if matches!(
                parent,
                Some(
                    VariableDeclarator(_)
                        | ObjectProperty(_)
                        | MethodDefinition(_)
                        | PropertyDefinition(_)
                        | AssignmentPattern(_)
                )
            ) {
                return None;
            }
            let name = if let Some(id) = id {
                label_text(id)
            } else if let Some(AssignmentExpression(a)) = parent
                && a.operator.is_assign()
            {
                a.left
                    .span()
                    .source_text(source)
                    .split_whitespace()
                    .collect()
            } else if let Some(ExportDefaultDeclaration(export)) = parent {
                span = export.span;
                "default".into()
            } else if kind == 4 {
                "<class>".into()
            } else if let Some(CallExpression(call)) = parent {
                self.call_names
                    .entry((call.span.start, call.span.end))
                    .or_insert_with(|| callback_name(call, source))
                    .clone()
                    .unwrap_or_else(|| "<function>".into())
            } else {
                "<function>".into()
            };
            return Some((name, kind, span));
        }
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
            CatchParameter(p) => (p.pattern.get_identifier_name()?.as_str(), 12),
            FormalParameter(p) if p.accessibility.is_some() || p.readonly || p.r#override => {
                (p.pattern.get_identifier_name()?.as_str(), 6)
            }
            SpreadElement(element) if matches!(parent, Some(ObjectExpression(_))) => (
                if let Expression::Identifier(id) = &element.argument {
                    id.name.as_str()
                } else {
                    "<unknown>"
                },
                6,
            ),
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
            ExportSpecifier(e) => (e.exported.span().source_text(source), 12),
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
fn finish_level(symbols: &mut Vec<Symbol>, kinds: &NavigationKinds) {
    let kind = |s: &Symbol| {
        kinds
            .get(&s.range)
            .copied()
            .unwrap_or(NavigationKind::Other)
    };
    symbols.sort_by(|a, b| {
        let ka = kind(a);
        let kb = kind(b);
        match (
            matches!(ka, NavigationKind::Constructor | NavigationKind::Spread),
            matches!(kb, NavigationKind::Constructor | NavigationKind::Spread),
        ) {
            (true, true) => ka.cmp(&kb),
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (false, false) => crate::outline_tree::compare_names(
                a.name.trim_matches(['\'', '"']),
                b.name.trim_matches(['\'', '"']),
            )
            .then(ka.cmp(&kb)),
        }
    });
    symbols.dedup_by(|later, first| {
        let k = kind(first);
        if first.name == later.name && k == kind(later) && k == NavigationKind::Declaration {
            first.children.append(&mut later.children);
            finish_level(&mut first.children, kinds);
            true
        } else {
            false
        }
    });
}
impl<'a> Visit<'a> for Outline<'a> {
    fn visit_spread_element(&mut self, spread: &oxc_ast::ast::SpreadElement<'a>) {
        if let Some((AstKind::ObjectExpression(object), _)) = self.ancestors.last() {
            if let Expression::Identifier(id) = &spread.argument
                && !self
                    .spread_names
                    .insert((object.span.start, id.name.as_str()))
            {
                return;
            }
            let node = AstKind::SpreadElement(self.alloc(spread));
            self.enter_node(node);
            self.leave_node(node);
        } else {
            oxc_ast_visit::walk::walk_spread_element(self, spread);
        }
    }
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
            let navigation_kind = match node {
                AstKind::ObjectProperty(p)
                    if !p.method && p.kind == oxc_ast::ast::PropertyKind::Init =>
                {
                    Some(if p.shorthand {
                        NavigationKind::Shorthand
                    } else {
                        NavigationKind::Property
                    })
                }
                AstKind::SpreadElement(_) => Some(NavigationKind::Spread),
                AstKind::MethodDefinition(m) if m.kind == MethodDefinitionKind::Constructor => {
                    Some(NavigationKind::Constructor)
                }
                AstKind::ExportSpecifier(_) => Some(NavigationKind::ExportSpecifier),
                AstKind::ArrowFunctionExpression(_) => Some(NavigationKind::Arrow),
                AstKind::Function(f) if f.r#type == FunctionType::FunctionExpression => {
                    Some(NavigationKind::Expression)
                }
                AstKind::Function(_) => Some(NavigationKind::Declaration),
                _ => None,
            };
            if let Some(k) = navigation_kind {
                self.kinds.insert(range, k);
            }
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
            finish_level(&mut symbol.children, &self.kinds);
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
        kinds: Default::default(),
        spread_names: Default::default(),
        call_names: Default::default(),
    };
    outline.visit_program(&parsed.program);
    finish_level(&mut outline.roots, &outline.kinds);
    outline.roots
}
