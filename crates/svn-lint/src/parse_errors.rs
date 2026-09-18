//! Errors the Svelte compiler's `parse()` throws on a template our own
//! parser reads without complaint.
//!
//! `svelte-check --tsgo` converts each component with svelte2tsx, which
//! calls the compiler's `parse()`; when that throws, the component
//! leaves the run entirely (no overlay, no diagnostics, not counted).
//! So the value of detecting these is the rejection itself; the code,
//! message and range are reproduced as the compiler reports them so the
//! lint pass can report the error where the file is linted on its own.
//!
//! The parser throws at the first error it meets while reading the
//! template front to back, and reads `<svelte:options>` last; the
//! findings are ordered the same way.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, ObjectPropertyKind, PropertyKey, Statement};
use oxc_span::{GetSpan, SourceType};
use svn_core::Range;
use svn_parser::ast::{
    AttrValuePart, Attribute, DirectiveKind, DirectiveValue, Fragment, Interpolation,
    InterpolationKind, Node, SvelteElementKind,
};

use crate::codes::Code;
use crate::messages;

/// One parse error: code, rendered message and range.
pub(crate) struct ParseFinding {
    pub code: Code,
    pub message: String,
    pub range: Range,
}

/// The first error the compiler's `parse()` throws on this template,
/// among the checks this module ports.
/// `ts` says whether the compiler reads the component as TypeScript.
pub(crate) fn first_template_parse_error(
    fragment: &Fragment,
    source: &str,
    ts: bool,
) -> Option<ParseFinding> {
    let mut finder = Finder {
        source,
        ts,
        found: None,
        meta_seen: Vec::new(),
    };
    finder.fragment(fragment, true);
    if finder.found.is_some() {
        return finder.found.map(|(_, f)| f);
    }
    read_options(fragment, source)
}

struct Finder<'s> {
    source: &'s str,
    ts: bool,
    /// The earliest finding so far, keyed by where the parser throws it.
    found: Option<(u32, ParseFinding)>,
    /// The root-only `<svelte:*>` tags met so far.
    meta_seen: Vec<SvelteElementKind>,
}

impl Finder<'_> {
    fn report(&mut self, at: u32, code: Code, message: String, range: Range) {
        if self.found.as_ref().is_none_or(|(key, _)| at < *key) {
            self.found = Some((
                at,
                ParseFinding {
                    code,
                    message,
                    range,
                },
            ));
        }
    }

    fn point(at: u32) -> Range {
        Range::new(at, at)
    }

    fn fragment(&mut self, fragment: &Fragment, at_root: bool) {
        for node in &fragment.nodes {
            self.node(node, at_root);
        }
    }

    fn node(&mut self, node: &Node, at_root: bool) {
        match node {
            Node::Element(el) => {
                self.tag_name(el.name.as_str(), el.range.start, false);
                self.attributes(&el.attributes);
                self.fragment(&el.children, false);
            }
            Node::Component(c) => {
                self.tag_name(c.name.as_str(), c.range.start, true);
                self.attributes(&c.attributes);
                self.fragment(&c.children, false);
            }
            Node::SvelteElement(se) => {
                let start = se.range.start;
                let root_only = matches!(
                    se.kind,
                    SvelteElementKind::Head
                        | SvelteElementKind::Options
                        | SvelteElementKind::Window
                        | SvelteElementKind::Document
                        | SvelteElementKind::Body
                );
                if root_only {
                    let name = format!("svelte:{}", se.kind.as_str());
                    if self.meta_seen.contains(&se.kind) {
                        self.report(
                            start,
                            Code::svelte_meta_duplicate,
                            messages::svelte_meta_duplicate(&name),
                            Self::point(start),
                        );
                    } else if !at_root {
                        self.report(
                            start,
                            Code::svelte_meta_invalid_placement,
                            messages::svelte_meta_invalid_placement(&name),
                            Self::point(start),
                        );
                    }
                    self.meta_seen.push(se.kind);
                }
                self.attributes(&se.attributes);
                let after_attributes = se
                    .attributes
                    .iter()
                    .map(|a| a.range().end)
                    .max()
                    .unwrap_or(start + 1);
                match se.kind {
                    SvelteElementKind::Component => {
                        self.component_this(&se.attributes, start, after_attributes)
                    }
                    SvelteElementKind::Element => {
                        self.element_this(&se.attributes, start, after_attributes)
                    }
                    _ => {}
                }
                self.fragment(&se.children, false);
            }
            Node::IfBlock(b) => {
                self.fragment(&b.consequent, false);
                for arm in &b.elseif_arms {
                    self.fragment(&arm.body, false);
                }
                if let Some(alt) = &b.alternate {
                    self.fragment(alt, false);
                }
            }
            Node::EachBlock(b) => {
                if let Some(clause) = &b.as_clause {
                    for r in [clause.context_range, clause.index_range]
                        .into_iter()
                        .flatten()
                    {
                        self.reserved_identifier(r);
                    }
                }
                self.fragment(&b.body, false);
                if let Some(alt) = &b.alternate {
                    self.fragment(alt, false);
                }
            }
            Node::AwaitBlock(b) => {
                if let Some(pending) = &b.pending {
                    self.fragment(pending, false);
                }
                if let Some(then) = &b.then_branch {
                    if let Some(r) = then.context_range {
                        self.reserved_identifier(r);
                    }
                    self.fragment(&then.body, false);
                }
                if let Some(catch) = &b.catch_branch {
                    if let Some(r) = catch.context_range {
                        self.reserved_identifier(r);
                    }
                    self.fragment(&catch.body, false);
                }
            }
            Node::KeyBlock(b) => self.fragment(&b.body, false),
            Node::SnippetBlock(b) => {
                // The snippet's name is read as an identifier.
                let tag = &self.source[b.range.start as usize..];
                if let Some(keyword) = tag.find("snippet") {
                    let after = &tag[keyword + "snippet".len()..];
                    let at = b.range.start
                        + (keyword + "snippet".len() + after.len() - after.trim_start().len())
                            as u32;
                    self.reserved_word(b.name.as_str(), at);
                }
                self.snippet_parameters(b);
                self.fragment(&b.body, false);
            }
            Node::Interpolation(i) => self.tag(i),
            Node::Text(_) | Node::Comment(_) => {}
        }
    }

    /// `tag_invalid_name`: an element or component name the compiler
    /// cannot read. Names with non-ASCII characters are left alone
    /// (their Unicode identifier classes are not modelled here).
    fn tag_name(&mut self, name: &str, start: u32, _component: bool) {
        if !name.is_ascii() || is_valid_tag_name(name) {
            return;
        }
        let range = Range::new(start + 1, start + 1 + name.len() as u32);
        self.report(
            start,
            Code::tag_invalid_name,
            messages::tag_invalid_name(),
            range,
        );
    }

    fn attributes(&mut self, attributes: &[Attribute]) {
        let mut unique: Vec<(u8, &str)> = Vec::new();
        for attr in attributes {
            let at = attr.range().start;
            self.attribute_value(attr);
            if let Attribute::Shorthand(s) = attr {
                let inner = &self.source[s.range.start as usize + 1..s.range.end as usize];
                let at = s.range.start + 1 + (inner.len() - inner.trim_start().len()) as u32;
                self.reserved_word(s.name.as_str(), at);
            }
            if let Attribute::Directive(d) = attr
                && d.kind != DirectiveKind::Style
                && let Some(DirectiveValue::Quoted(v)) = &d.value
            {
                let first = match v.parts.first() {
                    Some(AttrValuePart::Text { range }) => Some(range.start),
                    Some(AttrValuePart::Expression { range, .. }) if v.parts.len() > 1 => {
                        Some(range.start)
                    }
                    // An empty quoted value is one empty text chunk at
                    // the closing quote.
                    None => Some(v.range.end.saturating_sub(u32::from(v.quoted))),
                    Some(AttrValuePart::Expression { .. }) => None,
                };
                if let Some(first) = first {
                    self.report(
                        at,
                        Code::directive_invalid_value,
                        messages::directive_invalid_value(),
                        Self::point(first),
                    );
                }
            }
            // `bind:x` and `x` are the same attribute; `class:x` and
            // `style:x` are distinct from it and from each other.
            let key = match attr {
                Attribute::Plain(p) => Some((0, p.name.as_str())),
                Attribute::Expression(e) => Some((0, e.name.as_str())),
                Attribute::Shorthand(s) => Some((0, s.name.as_str())),
                Attribute::Directive(d) => match d.kind {
                    DirectiveKind::Bind => Some((0, d.name.as_str())),
                    DirectiveKind::Class => Some((1, d.name.as_str())),
                    DirectiveKind::Style => Some((2, d.name.as_str())),
                    _ => None,
                },
                Attribute::Spread(_) | Attribute::Comment(_) => None,
            };
            if let Some(key) = key {
                if unique.contains(&key) {
                    self.report(
                        at,
                        Code::attribute_duplicate,
                        messages::attribute_duplicate(),
                        attr.range(),
                    );
                } else if key.1 != "this" {
                    unique.push(key);
                }
            }
        }
    }

    /// `{#…}` / `{@…}` inside an attribute value, or an empty `{}`.
    fn attribute_value(&mut self, attr: &Attribute) {
        let part_expressions = |parts: &[AttrValuePart]| -> Vec<Range> {
            parts
                .iter()
                .filter_map(|part| match part {
                    AttrValuePart::Expression {
                        expression_range, ..
                    } => Some(*expression_range),
                    AttrValuePart::Text { .. } => None,
                })
                .collect()
        };
        let expressions: Vec<Range> = match attr {
            Attribute::Plain(p) => p
                .value
                .as_ref()
                .map(|v| part_expressions(&v.parts))
                .unwrap_or_default(),
            Attribute::Expression(e) => vec![e.expression_range],
            Attribute::Directive(d) => match &d.value {
                Some(DirectiveValue::Expression {
                    expression_range, ..
                }) => vec![*expression_range],
                Some(DirectiveValue::BindPair {
                    getter_range,
                    setter_range,
                    ..
                }) => vec![Range::new(getter_range.start, setter_range.end)],
                Some(DirectiveValue::Quoted(v)) => part_expressions(&v.parts),
                None => Vec::new(),
            },
            _ => Vec::new(),
        };
        for expression in &expressions {
            if expression.slice(self.source).trim().is_empty() {
                let at = expression.end;
                self.report(
                    at,
                    Code::js_parse_error,
                    "Unexpected token\nhttps://svelte.dev/e/js_parse_error".to_string(),
                    Self::point(at),
                );
            }
        }
        let braces: Vec<u32> = expressions
            .iter()
            .map(|e| e.start.saturating_sub(1))
            .collect();
        let bytes = self.source.as_bytes();
        for brace in braces {
            if bytes.get(brace as usize) != Some(&b'{') {
                continue;
            }
            let sigil = bytes.get(brace as usize + 1).copied();
            if !matches!(sigil, Some(b'#' | b'@')) {
                continue;
            }
            let name_start = brace as usize + 2;
            let name_end = self.source[name_start..]
                .find(|c: char| !c.is_ascii_lowercase())
                .map_or(self.source.len(), |i| name_start + i);
            let name = &self.source[name_start..name_end];
            let (code, message) = if sigil == Some(b'#') {
                (
                    Code::block_invalid_placement,
                    messages::block_invalid_placement(name, "in attribute value"),
                )
            } else {
                (
                    Code::tag_invalid_placement,
                    messages::tag_invalid_placement(name, "in attribute value"),
                )
            };
            self.report(brace, code, message, Self::point(brace));
        }
    }

    /// A snippet's parameter list is read as an arrow function's.
    fn snippet_parameters(&mut self, b: &svn_parser::ast::SnippetBlock) {
        let params = b.parameters_range.slice(self.source);
        let generics = b
            .generics_range
            .map(|g| format!("<{}>", g.slice(self.source)))
            .unwrap_or_default();
        let text = format!("{generics}({params}) => {{}}");
        let alloc = Allocator::default();
        let source_type = SourceType::default()
            .with_module(true)
            .with_typescript(self.ts);
        let parsed = oxc_parser::Parser::new(&alloc, &text, source_type).parse_expression();
        if let Err(errors) = parsed {
            let message = errors
                .first()
                .map_or_else(|| "Unexpected token".to_string(), |e| e.message.to_string());
            let at = b.parameters_range.start;
            self.report(
                at,
                Code::js_parse_error,
                format!("{message}\nhttps://svelte.dev/e/js_parse_error"),
                Self::point(at),
            );
        }
    }

    /// The first plain / expression / shorthand attribute named `this`.
    fn this_attribute(attributes: &[Attribute]) -> Option<&Attribute> {
        attributes.iter().find(|a| match a {
            Attribute::Plain(p) => p.name == "this",
            Attribute::Expression(e) => e.name == "this",
            Attribute::Shorthand(s) => s.name == "this",
            _ => false,
        })
    }

    fn component_this(&mut self, attributes: &[Attribute], start: u32, at: u32) {
        match Self::this_attribute(attributes) {
            None => self.report(
                at,
                Code::svelte_component_missing_this,
                messages::svelte_component_missing_this(),
                Self::point(start),
            ),
            Some(attr) if !is_expression_attribute(attr) => self.report(
                at,
                Code::svelte_component_invalid_this,
                messages::svelte_component_invalid_this(),
                Self::point(attr.range().start),
            ),
            Some(_) => {}
        }
    }

    fn element_this(&mut self, attributes: &[Attribute], start: u32, at: u32) {
        match Self::this_attribute(attributes) {
            None => self.report(
                at,
                Code::svelte_element_missing_this,
                messages::svelte_element_missing_this(),
                Self::point(start),
            ),
            Some(Attribute::Plain(p)) if p.value.is_none() => self.report(
                at,
                Code::svelte_element_missing_this,
                messages::svelte_element_missing_this(),
                p.range,
            ),
            Some(_) => {}
        }
    }

    /// A pattern that is a plain identifier must not be a reserved word.
    fn reserved_identifier(&mut self, range: Range) {
        let text = range.slice(self.source);
        let trimmed = text.trim_start();
        let start = range.start + (text.len() - trimmed.len()) as u32;
        let end = trimmed
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .unwrap_or(trimmed.len());
        self.reserved_word(&trimmed[..end], start);
    }

    fn reserved_word(&mut self, word: &str, at: u32) {
        if RESERVED_WORDS.contains(&word) {
            self.report(
                at,
                Code::unexpected_reserved_word,
                messages::unexpected_reserved_word(word),
                Self::point(at),
            );
        }
    }

    fn tag(&mut self, i: &Interpolation) {
        let start = i.range.start;
        match i.kind {
            InterpolationKind::AtTag => {
                // `special()` reads `html`, `debug`, `const` and
                // `render`; anything else after the `@` is an error.
                if let Some(at) = self.source[start as usize..i.range.end as usize].find('@') {
                    let at = start + at as u32 + 1;
                    self.report(
                        at,
                        Code::expected_tag,
                        messages::expected_tag(),
                        Self::point(at),
                    );
                }
            }
            InterpolationKind::AtDebug => self.debug_tag(i),
            InterpolationKind::AtRender => {
                let range = trimmed_range(i.expression_range, self.source);
                let is_call =
                    parse_expression(range, self.source, |expr| match unparenthesized(expr) {
                        Expression::CallExpression(_) => true,
                        Expression::ChainExpression(chain) => matches!(
                            chain.expression,
                            oxc_ast::ast::ChainElement::CallExpression(_)
                        ),
                        _ => false,
                    });
                if is_call == Some(false) {
                    self.report(
                        range.start,
                        Code::render_tag_invalid_expression,
                        messages::render_tag_invalid_expression(),
                        range,
                    );
                }
            }
            InterpolationKind::AtConst => self.const_tag(i),
            InterpolationKind::Expression => {
                // An empty `{}` has no expression for acorn to read: it
                // stops at the closing brace.
                if i.expression_range.slice(self.source).trim().is_empty() {
                    let at = i.expression_range.end;
                    self.report(
                        at,
                        Code::js_parse_error,
                        "Unexpected token\nhttps://svelte.dev/e/js_parse_error".to_string(),
                        Self::point(at),
                    );
                }
            }
            InterpolationKind::DeclConst
            | InterpolationKind::DeclLet
            | InterpolationKind::AtHtml => {}
        }
    }

    fn debug_tag(&mut self, i: &Interpolation) {
        let range = i.expression_range;
        if range.slice(self.source).trim().is_empty() {
            return;
        }
        let offending = parse_expression(range, self.source, |expr| {
            let is_identifier =
                |e: &Expression<'_>| matches!(unparenthesized(e), Expression::Identifier(_));
            match unparenthesized(expr) {
                Expression::SequenceExpression(seq) => seq
                    .expressions
                    .iter()
                    .find(|e| !is_identifier(e))
                    .map(|e| expression_start(e)),
                e if !is_identifier(e) => Some(expression_start(e)),
                _ => None,
            }
        })
        .flatten();
        if let Some(offset) = offending {
            let at = range.start + offset;
            self.report(
                at,
                Code::debug_tag_invalid_arguments,
                messages::debug_tag_invalid_arguments(),
                Self::point(at),
            );
        }
    }

    /// `{@const a = b, c = d}`: the initializer read after the first
    /// `=` is a comma sequence not wrapped in parentheses.
    fn const_tag(&mut self, i: &Interpolation) {
        let range = i.expression_range;
        let text = range.slice(self.source);
        let wrapped = format!("const {text};");
        let alloc = Allocator::default();
        let parsed = oxc_parser::Parser::new(&alloc, &wrapped, SourceType::ts()).parse();
        let Some(Statement::VariableDeclaration(decl)) = parsed.program.body.first() else {
            return;
        };
        if decl.declarations.len() < 2 {
            return;
        }
        let Some(init) = &decl.declarations[0].init else {
            return;
        };
        let prefix = "const ".len() as u32;
        let init_start = range.start + init.span().start - prefix;
        let end = trimmed_range(range, self.source).end;
        self.report(
            init_start,
            Code::const_tag_invalid_expression,
            messages::const_tag_invalid_expression(),
            Range::new(init_start, end),
        );
    }
}

fn expression_start(e: &Expression<'_>) -> u32 {
    unparenthesized(e).span().start
}

fn unparenthesized<'a, 'b>(mut expr: &'a Expression<'b>) -> &'a Expression<'b> {
    while let Expression::ParenthesizedExpression(p) = expr {
        expr = &p.expression;
    }
    expr
}

fn trimmed_range(range: Range, source: &str) -> Range {
    let text = range.slice(source);
    let start = range.start + (text.len() - text.trim_start().len()) as u32;
    let end = range.end - (text.len() - text.trim_end().len()) as u32;
    Range::new(start, end.max(start))
}

/// Parse the template expression at `range` (offsets in the result are
/// relative to `range.start`); `None` when it does not parse.
fn parse_expression<R>(
    range: Range,
    source: &str,
    f: impl FnOnce(&Expression<'_>) -> R,
) -> Option<R> {
    let text = source.get(range.start as usize..range.end as usize)?;
    let alloc = Allocator::default();
    let source_type = SourceType::default()
        .with_module(true)
        .with_typescript(true);
    let expr = oxc_parser::Parser::new(&alloc, text, source_type)
        .parse_expression()
        .ok()?;
    Some(f(&expr))
}

/// The compiler's `is_expression_attribute`.
fn is_expression_attribute(attr: &Attribute) -> bool {
    match attr {
        Attribute::Expression(_) | Attribute::Shorthand(_) => true,
        Attribute::Plain(p) => matches!(
            p.value.as_ref().map(|v| v.parts.as_slice()),
            Some([AttrValuePart::Expression { .. }])
        ),
        _ => false,
    }
}

/// `is_valid_element_name` or `regex_valid_component_name`, for an
/// ASCII name.
fn is_valid_tag_name(name: &str) -> bool {
    let alnum = |b: u8| b.is_ascii_alphanumeric();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'$';
    let starts_alpha = |s: &str| s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
    // `!DOCTYPE`
    if let Some(rest) = name.strip_prefix('!') {
        return !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphabetic());
    }
    // `svelte:head`, `enhanced:img`: `[a-zA-Z][a-zA-Z0-9]*:` then
    // `[a-zA-Z][a-zA-Z0-9-]*[a-zA-Z0-9]`.
    if let Some((ns, local)) = name.split_once(':')
        && starts_alpha(ns)
        && ns.bytes().all(alnum)
        && local.len() >= 2
        && starts_alpha(local)
        && local.bytes().all(|b| alnum(b) || b == b'-')
        && local.bytes().last().is_some_and(alnum)
    {
        return true;
    }
    // Element: `[a-zA-Z][a-zA-Z0-9]*(-[a-zA-Z0-9.\-_]+)*`.
    if starts_alpha(name) {
        let head_end = name.bytes().position(|b| !alnum(b)).unwrap_or(name.len());
        let rest = &name[head_end..];
        if rest.is_empty()
            || (rest.starts_with('-')
                && rest.len() >= 2
                && rest
                    .bytes()
                    .all(|b| alnum(b) || matches!(b, b'.' | b'-' | b'_')))
        {
            return true;
        }
    }
    // Component: an uppercase letter then identifier characters and
    // dots, or a dotted path of identifiers.
    if name.as_bytes().first().is_some_and(u8::is_ascii_uppercase)
        && name.bytes().all(|b| ident(b) || b == b'.')
    {
        return true;
    }
    let mut segments = name.split('.');
    let first_ok = segments
        .next()
        .is_some_and(|s| starts_alpha(s) && s.bytes().all(ident));
    let rest: Vec<&str> = segments.collect();
    first_ok && !rest.is_empty() && rest.iter().all(|s| !s.is_empty() && s.bytes().all(ident))
}

/// `RESERVED_WORDS` from the compiler's `utils.js`.
const RESERVED_WORDS: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// A `<svelte:options>` attribute's value the way `get_static_value`
/// reads it.
enum StaticValue {
    Bool,
    Text(String),
    Null,
    /// A literal of another kind (number, …).
    OtherLiteral,
    /// Not static (`null` in the compiler).
    Dynamic,
}

fn static_value(attr: &Attribute, source: &str) -> StaticValue {
    let expression = match attr {
        Attribute::Plain(p) => match &p.value {
            None => return StaticValue::Bool,
            Some(v) => match v.parts.as_slice() {
                [] if v.quoted => return StaticValue::Text(String::new()),
                [] => return StaticValue::Bool,
                [AttrValuePart::Text { range }] => {
                    return StaticValue::Text(range.slice(source).to_string());
                }
                [
                    AttrValuePart::Expression {
                        expression_range, ..
                    },
                ] => *expression_range,
                _ => return StaticValue::Dynamic,
            },
        },
        Attribute::Expression(e) => e.expression_range,
        Attribute::Shorthand(_) => return StaticValue::Dynamic,
        _ => return StaticValue::Dynamic,
    };
    parse_expression(expression, source, |expr| match unparenthesized(expr) {
        Expression::BooleanLiteral(_) => StaticValue::Bool,
        Expression::StringLiteral(s) => StaticValue::Text(s.value.to_string()),
        Expression::NullLiteral(_) => StaticValue::Null,
        Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::RegExpLiteral(_) => StaticValue::OtherLiteral,
        _ => StaticValue::Dynamic,
    })
    .unwrap_or(StaticValue::Dynamic)
}

/// `read_options`, run once the whole template has been read: the
/// first `<svelte:options>` at the root must have static, known
/// attributes with valid values, and no children.
fn read_options(fragment: &Fragment, source: &str) -> Option<ParseFinding> {
    let options = fragment.nodes.iter().find_map(|n| match n {
        Node::SvelteElement(se) if se.kind == SvelteElementKind::Options => Some(se),
        _ => None,
    })?;
    let finding = |code: Code, message: String, range: Range| {
        Some(ParseFinding {
            code,
            message,
            range,
        })
    };
    for attr in &options.attributes {
        let range = attr.range();
        let name = match attr {
            Attribute::Plain(p) => p.name.as_str(),
            Attribute::Expression(e) => e.name.as_str(),
            Attribute::Shorthand(s) => s.name.as_str(),
            Attribute::Comment(_) => continue,
            Attribute::Directive(_) | Attribute::Spread(_) => {
                return finding(
                    Code::svelte_options_invalid_attribute,
                    messages::svelte_options_invalid_attribute(),
                    range,
                );
            }
        };
        let invalid_value = |list: &str| {
            finding(
                Code::svelte_options_invalid_attribute_value,
                messages::svelte_options_invalid_attribute_value(list),
                range,
            )
        };
        match name {
            "runes" | "immutable" | "preserveWhitespace" | "accessors" => {
                if !matches!(static_value(attr, source), StaticValue::Bool) {
                    return invalid_value("true or false");
                }
            }
            "tag" => {
                return finding(
                    Code::svelte_options_deprecated_tag,
                    messages::svelte_options_deprecated_tag(),
                    range,
                );
            }
            "customElement" => {
                if let Some(f) = custom_element(attr, source) {
                    return Some(f);
                }
            }
            "namespace" => {
                let valid = matches!(
                    static_value(attr, source),
                    StaticValue::Text(ref t) if matches!(
                        t.as_str(),
                        "html" | "mathml" | "svg" | "http://www.w3.org/2000/svg" | "http://www.w3.org/1998/Math/MathML"
                    )
                );
                if !valid {
                    return invalid_value("\"html\", \"mathml\" or \"svg\"");
                }
            }
            "css" => {
                if !matches!(static_value(attr, source), StaticValue::Text(ref t) if t == "injected")
                {
                    return invalid_value("\"injected\"");
                }
            }
            other => {
                return finding(
                    Code::svelte_options_unknown_attribute,
                    messages::svelte_options_unknown_attribute(other),
                    range,
                );
            }
        }
    }
    if let (Some(first), Some(last)) = (
        options.children.nodes.first(),
        options.children.nodes.last(),
    ) {
        return finding(
            Code::svelte_meta_invalid_content,
            messages::svelte_meta_invalid_content("svelte:options"),
            Range::new(first.range().start, last.range().end),
        );
    }
    None
}

/// The `customElement` option: a valid tag name, `null`, or an object
/// literal of static `tag` / `props` / `shadow` / `extend` entries.
fn custom_element(attr: &Attribute, source: &str) -> Option<ParseFinding> {
    let range = attr.range();
    let invalid = |code: Code, message: String| {
        Some(ParseFinding {
            code,
            message,
            range,
        })
    };
    let invalid_ce = || {
        invalid(
            Code::svelte_options_invalid_customelement,
            messages::svelte_options_invalid_customelement(),
        )
    };
    let expression = match attr {
        Attribute::Plain(p) => match &p.value {
            None => return invalid_ce(),
            Some(v) => match v.parts.first() {
                Some(AttrValuePart::Expression {
                    expression_range, ..
                }) => *expression_range,
                // A text value is the tag name itself.
                _ => {
                    let tag = match static_value(attr, source) {
                        StaticValue::Text(t) => Some(t),
                        _ => None,
                    };
                    return validate_tag(tag.as_deref(), range);
                }
            },
        },
        Attribute::Expression(e) => e.expression_range,
        _ => return invalid_ce(),
    };
    enum Shape {
        Null,
        NotObject,
        BadProperty,
        Object {
            tag: Option<Option<String>>,
            props_ok: Option<bool>,
            shadow_ok: Option<bool>,
        },
    }
    let shape = parse_expression(expression, source, |expr| {
        let obj = match unparenthesized(expr) {
            Expression::NullLiteral(_) => return Shape::Null,
            Expression::ObjectExpression(obj) => obj,
            _ => return Shape::NotObject,
        };
        let mut tag = None;
        let mut props_ok = None;
        let mut shadow_ok = None;
        for prop in &obj.properties {
            let ObjectPropertyKind::ObjectProperty(p) = prop else {
                return Shape::BadProperty;
            };
            let PropertyKey::StaticIdentifier(key) = &p.key else {
                return Shape::BadProperty;
            };
            if p.computed {
                return Shape::BadProperty;
            }
            match key.name.as_str() {
                "tag" if tag.is_none() => {
                    tag = Some(match &p.value {
                        Expression::StringLiteral(s) => Some(s.value.to_string()),
                        _ => None,
                    });
                }
                "props" if props_ok.is_none() => props_ok = Some(valid_props(&p.value)),
                "shadow" if shadow_ok.is_none() => {
                    shadow_ok = Some(match &p.value {
                        Expression::StringLiteral(s) => matches!(s.value.as_str(), "open" | "none"),
                        Expression::ObjectExpression(_) => true,
                        _ => false,
                    });
                }
                _ => {}
            }
        }
        Shape::Object {
            tag,
            props_ok,
            shadow_ok,
        }
    })
    .unwrap_or(Shape::NotObject);
    match shape {
        Shape::Null => None,
        Shape::NotObject | Shape::BadProperty => invalid_ce(),
        Shape::Object {
            tag,
            props_ok,
            shadow_ok,
        } => {
            if let Some(tag) = tag
                && let Some(f) = validate_tag(tag.as_deref(), range)
            {
                // The compiler reports this one against the property
                // pair rather than a node, so it carries no position.
                return Some(f);
            }
            if props_ok == Some(false) {
                return invalid(
                    Code::svelte_options_invalid_customelement_props,
                    messages::svelte_options_invalid_customelement_props(),
                );
            }
            if shadow_ok == Some(false) {
                return invalid(
                    Code::svelte_options_invalid_customelement_shadow,
                    messages::svelte_options_invalid_customelement_shadow(),
                );
            }
            None
        }
    }
}

/// The `props` entry of a `customElement` object: an object of objects
/// whose entries are static `type` / `reflect` / `attribute` literals.
fn valid_props(value: &Expression<'_>) -> bool {
    let Expression::ObjectExpression(props) = value else {
        return false;
    };
    props.properties.iter().all(|prop| {
        let ObjectPropertyKind::ObjectProperty(p) = prop else {
            return false;
        };
        let (PropertyKey::StaticIdentifier(_), false, Expression::ObjectExpression(inner)) =
            (&p.key, p.computed, &p.value)
        else {
            return false;
        };
        inner.properties.iter().all(|prop| {
            let ObjectPropertyKind::ObjectProperty(p) = prop else {
                return false;
            };
            let PropertyKey::StaticIdentifier(key) = &p.key else {
                return false;
            };
            if p.computed {
                return false;
            }
            match (key.name.as_str(), &p.value) {
                ("type", Expression::StringLiteral(s)) => {
                    matches!(
                        s.value.as_str(),
                        "String" | "Number" | "Boolean" | "Array" | "Object"
                    )
                }
                ("reflect", Expression::BooleanLiteral(_)) => true,
                ("attribute", Expression::StringLiteral(_)) => true,
                _ => false,
            }
        })
    })
}

/// `validate_tag`: a custom element name, lowercase and hyphenated, not
/// one of the reserved names.
fn validate_tag(tag: Option<&str>, range: Range) -> Option<ParseFinding> {
    let invalid = |code: Code, message: String| {
        Some(ParseFinding {
            code,
            message,
            range,
        })
    };
    let Some(tag) = tag else {
        return invalid(
            Code::svelte_options_invalid_tagname,
            messages::svelte_options_invalid_tagname(),
        );
    };
    if tag.is_empty() {
        return None;
    }
    let name_char = |c: char| {
        c.is_ascii_lowercase()
            || c.is_ascii_digit()
            || matches!(c, '_' | '.' | '-')
            || (c as u32) >= 0xB7
    };
    let valid = tag.starts_with(|c: char| c.is_ascii_lowercase())
        && tag.contains('-')
        && tag.chars().all(name_char);
    if !valid {
        return invalid(
            Code::svelte_options_invalid_tagname,
            messages::svelte_options_invalid_tagname(),
        );
    }
    const RESERVED: &[&str] = &[
        "annotation-xml",
        "color-profile",
        "font-face",
        "font-face-src",
        "font-face-uri",
        "font-face-format",
        "font-face-name",
        "missing-glyph",
    ];
    if RESERVED.contains(&tag) {
        return invalid(
            Code::svelte_options_reserved_tagname,
            messages::svelte_options_reserved_tagname(),
        );
    }
    None
}
