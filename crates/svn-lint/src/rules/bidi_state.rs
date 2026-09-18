//! Which string literals and template chunks the compiler reports for
//! `bidirectional_control_characters`.
//!
//! The compiler tests each string literal's value and each template
//! chunk's cooked value with one shared global regular expression
//! (`regex_bidirectional_control_characters`, flag `g`). A global
//! regex's `test` searches from its `lastIndex`: a match moves
//! `lastIndex` to the end of the match, a miss resets it to 0. So a
//! value is only searched from where the previous tested value's match
//! ended, and a bidi character before that point goes unreported. Text
//! nodes set `lastIndex` back to 0 before their own search.
//!
//! The tested values come in the compiler's walk order: the module
//! script, the instance script, then the template (a template
//! literal's substitutions before its chunks). [`replay`] records that
//! sequence and runs it to find the literals that warn.
//!
//! The regex is a module global of the compiler, so its `lastIndex`
//! also carries from one compiled component to the next: svelte-check
//! compiles every component in one process, and a component starts
//! from wherever the previous one left the regex. [`BidiTrace`] is what
//! one component does to it, for the caller to run the components in
//! the compiler's order.

use std::collections::HashSet;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Class, ClassElement, Declaration, ExportFromDeclaration, ExportNamedDeclaration, Function,
    FunctionType, ImportDeclaration, ImportOrExportKind, MethodDefinitionType, Program,
    PropertyDefinitionType, Statement, StringLiteral, TSType, TemplateLiteral, VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::SourceType;
use smol_str::SmolStr;
use svn_analyze::template_scope::{TemplateScopeVisitor, walk_with_visitor};
use svn_core::Range;
use svn_parser::ast::{AttrValuePart, Attribute, DirectiveValue, Fragment, Node};
use svn_parser::{Component, Document, Element, SvelteElement};

/// The literals and template chunks the compiler warns on (their start
/// offsets) when the component starts with the regex at `last_index`,
/// or `None` when the component cannot hold a bidi character at all (no
/// such character, no `\u` escape); and what the component does to the
/// regex.
pub(crate) fn replay(
    doc: &Document<'_>,
    fragment: Option<&Fragment>,
    source: &str,
    module_program: Option<&Program<'_>>,
    instance_program: Option<&Program<'_>>,
    last_index: u32,
) -> (Option<HashSet<u32>>, BidiTrace) {
    let may_hold_bidi = source.chars().any(is_bidi) || source.contains("\\u");
    // The compiler parses the template with its trailing whitespace
    // trimmed, so text there is never tested.
    let template_end = source.trim_end().len() as u32;
    if !may_hold_bidi {
        // Every search fails and resets the regex; only whether any
        // happens matters. Text at the top level almost always settles
        // it without a walk.
        let top_level_text = fragment.is_some_and(|f| {
            f.nodes
                .iter()
                .any(|n| matches!(n, Node::Text(t) if t.range.start < template_end))
        });
        if top_level_text {
            return (None, BidiTrace::Resets);
        }
    }
    let mut uses = Vec::new();
    for (script, program) in [
        (doc.module_script.as_ref(), module_program),
        (doc.instance_script.as_ref(), instance_program),
    ] {
        if let (Some(script), Some(program)) = (script, program) {
            let mut tester = Tester {
                uses: &mut uses,
                base: script.content_range.start,
            };
            tester.visit_program(program);
        }
    }
    if let Some(fragment) = fragment {
        let mut template = TemplateTester {
            source,
            template_end,
            ts: crate::rules::typescript_features::compiler_parses_as_ts(source),
            allocator: Allocator::default(),
            uses: &mut uses,
        };
        walk_with_visitor(fragment, source, &mut template);
    }
    if !may_hold_bidi {
        let trace = if uses.is_empty() {
            BidiTrace::Untouched
        } else {
            BidiTrace::Resets
        };
        return (None, trace);
    }
    let trace = BidiTrace::Uses(uses);
    let (_, warned) = trace.run(last_index);
    (Some(warned.into_iter().collect()), trace)
}

fn is_bidi(c: char) -> bool {
    matches!(c as u32, 0x202A..=0x202E | 0x2066..=0x2069)
}

/// What compiling one component does to the compiler's bidi regex.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BidiTrace {
    /// Nothing is tested: the regex keeps its state.
    #[default]
    Untouched,
    /// Values are tested, none can hold a bidi character: the regex
    /// ends reset.
    Resets,
    /// The searches in order.
    Uses(Vec<RegexUse>),
}

/// One use of the regex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegexUse {
    /// A text node sets `lastIndex` to 0 and searches a copy.
    Text,
    /// `test` on the value of the node starting at `start`; `groups`
    /// are the runs of bidi characters in the value (UTF-16 ranges).
    Test { start: u32, groups: Vec<(u32, u32)> },
}

impl BidiTrace {
    /// Run the component's searches from `last_index`: where the regex
    /// ends, and the start offsets of the nodes whose test matches.
    pub fn run(&self, mut last_index: u32) -> (u32, Vec<u32>) {
        let mut warned = Vec::new();
        match self {
            BidiTrace::Untouched => {}
            BidiTrace::Resets => last_index = 0,
            BidiTrace::Uses(uses) => {
                for use_ in uses {
                    match use_ {
                        RegexUse::Text => last_index = 0,
                        RegexUse::Test { start, groups } => {
                            // A match starts at or after `lastIndex` and
                            // runs to the end of its group.
                            match groups.iter().find(|&&(_, end)| end > last_index) {
                                Some(&(_, end)) => {
                                    last_index = end;
                                    warned.push(*start);
                                }
                                None => last_index = 0,
                            }
                        }
                    }
                }
            }
        }
        (last_index, warned)
    }
}

fn record_test(uses: &mut Vec<RegexUse>, value: &str, start: u32) {
    let mut groups: Vec<(u32, u32)> = Vec::new();
    let mut offset = 0u32;
    for c in value.chars() {
        if is_bidi(c) {
            // Bidi characters are one UTF-16 unit each.
            match groups.last_mut() {
                Some(last) if last.1 == offset => last.1 += 1,
                _ => groups.push((offset, offset + 1)),
            }
        }
        offset += c.len_utf16() as u32;
    }
    uses.push(RegexUse::Test { start, groups });
}

struct Tester<'s> {
    uses: &'s mut Vec<RegexUse>,
    base: u32,
}

impl<'a> Visit<'a> for Tester<'_> {
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        record_test(self.uses, &it.value, self.base + it.span.start);
    }

    fn visit_template_literal(&mut self, it: &TemplateLiteral<'a>) {
        for expr in &it.expressions {
            self.visit_expression(expr);
        }
        for quasi in &it.quasis {
            let cooked = quasi.value.cooked.as_deref().unwrap_or_default();
            record_test(self.uses, cooked, self.base + quasi.span.start);
        }
    }

    // What the compiler strips from TypeScript before analysis is never
    // tested.
    fn visit_ts_type(&mut self, _it: &TSType<'a>) {}

    fn visit_statement(&mut self, it: &Statement<'a>) {
        let stripped = match it {
            Statement::TSTypeAliasDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSExternalModuleDeclaration(_)
            | Statement::TSNamespaceDeclaration(_)
            | Statement::TSGlobalDeclaration(_) => true,
            Statement::ExportDeclaration(d) => matches!(
                d.declaration,
                Declaration::TSTypeAliasDeclaration(_)
                    | Declaration::TSInterfaceDeclaration(_)
                    | Declaration::TSExternalModuleDeclaration(_)
                    | Declaration::TSNamespaceDeclaration(_)
                    | Declaration::TSGlobalDeclaration(_)
            ),
            _ => false,
        };
        if !stripped {
            walk::walk_statement(self, it);
        }
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        if it.import_kind != ImportOrExportKind::Type {
            walk::walk_import_declaration(self, it);
        }
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        if it.export_kind != ImportOrExportKind::Type {
            walk::walk_export_named_declaration(self, it);
        }
    }

    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        if it.export_kind != ImportOrExportKind::Type {
            walk::walk_export_from_declaration(self, it);
        }
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if !it.declare {
            walk::walk_variable_declaration(self, it);
        }
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        if !it.declare {
            walk::walk_class(self, it);
        }
    }

    fn visit_class_element(&mut self, it: &ClassElement<'a>) {
        let stripped = match it {
            ClassElement::MethodDefinition(m) => {
                m.r#type == MethodDefinitionType::TSAbstractMethodDefinition
            }
            ClassElement::PropertyDefinition(p) => {
                p.declare || p.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition
            }
            ClassElement::TSIndexSignature(_) => true,
            ClassElement::StaticBlock(_) | ClassElement::AccessorProperty(_) => false,
        };
        if !stripped {
            walk::walk_class_element(self, it);
        }
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        if it.r#type != FunctionType::TSDeclareFunction {
            walk::walk_function(self, it, flags);
        }
    }
}

struct TemplateTester<'s> {
    source: &'s str,
    /// Where the template the compiler parses ends.
    template_end: u32,
    ts: bool,
    allocator: Allocator,
    uses: &'s mut Vec<RegexUse>,
}

impl TemplateTester<'_> {
    fn expression(&mut self, range: Range) {
        let Some(text) = self.source.get(range.start as usize..range.end as usize) else {
            return;
        };
        let source_type = SourceType::default()
            .with_module(true)
            .with_typescript(self.ts);
        self.allocator.reset();
        if let Ok(expr) = Parser::new(&self.allocator, text, source_type).parse_expression() {
            let mut tester = Tester {
                uses: self.uses,
                base: range.start,
            };
            tester.visit_expression(&expr);
        }
    }

    fn declaration(&mut self, range: Range) {
        let Some(text) = self.source.get(range.start as usize..range.end as usize) else {
            return;
        };
        let source_type = SourceType::default()
            .with_module(true)
            .with_typescript(self.ts);
        self.allocator.reset();
        // `{@const NAME = EXPR}`: read the body behind a `let `.
        let wrapped = format!("let {text}");
        let parsed = Parser::new(&self.allocator, &wrapped, source_type).parse();
        let mut tester = Tester {
            uses: self.uses,
            base: range.start.wrapping_sub(4),
        };
        tester.visit_program(&parsed.program);
    }

    fn attributes(&mut self, attributes: &[Attribute]) {
        for attr in attributes {
            match attr {
                Attribute::Plain(p) => {
                    if let Some(v) = &p.value {
                        self.parts(&v.parts);
                    }
                }
                Attribute::Expression(e) => self.expression(e.expression_range),
                Attribute::Spread(s) => self.expression(s.expression_range),
                Attribute::Directive(d) => match &d.value {
                    Some(DirectiveValue::Expression {
                        expression_range, ..
                    }) => self.expression(*expression_range),
                    Some(DirectiveValue::Quoted(v)) => self.parts(&v.parts),
                    Some(DirectiveValue::BindPair { .. }) | None => {}
                },
                Attribute::Shorthand(_) | Attribute::Comment(_) => {}
            }
        }
    }

    fn parts(&mut self, parts: &[AttrValuePart]) {
        for part in parts {
            match part {
                AttrValuePart::Text { .. } => self.uses.push(RegexUse::Text),
                AttrValuePart::Expression {
                    expression_range, ..
                } => self.expression(*expression_range),
            }
        }
    }
}

impl TemplateScopeVisitor for TemplateTester<'_> {
    fn visit_expr(&mut self, range: Range) {
        self.expression(range);
    }

    fn visit_at_const(&mut self, _bound_names: &[SmolStr], expr_range: Range) {
        self.declaration(expr_range);
    }

    fn visit_text(&mut self, range: Range) {
        if range.start < self.template_end {
            self.uses.push(RegexUse::Text);
        }
    }

    fn visit_element(&mut self, element: &Element) {
        self.attributes(&element.attributes);
    }

    fn visit_component(&mut self, component: &Component) {
        self.attributes(&component.attributes);
    }

    fn visit_svelte_element(&mut self, element: &SvelteElement) {
        self.attributes(&element.attributes);
    }
}
