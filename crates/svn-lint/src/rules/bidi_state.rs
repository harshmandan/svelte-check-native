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
//! literal's substitutions before its chunks). [`compiler_warned`]
//! replays that sequence and returns the literals that warn.

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
use svn_parser::ast::{AttrValuePart, Attribute, DirectiveValue, Fragment};
use svn_parser::{Component, Document, Element, SvelteElement};

/// Start offsets of the literals and template chunks the compiler
/// warns on, or `None` when the component cannot hold a bidi
/// character at all (no such character, no `\u` escape).
pub(crate) fn compiler_warned(
    doc: &Document<'_>,
    fragment: Option<&Fragment>,
    source: &str,
    module_program: Option<&Program<'_>>,
    instance_program: Option<&Program<'_>>,
) -> Option<HashSet<u32>> {
    if !source.chars().any(is_bidi) && !source.contains("\\u") {
        return None;
    }
    let mut state = State::default();
    for (script, program) in [
        (doc.module_script.as_ref(), module_program),
        (doc.instance_script.as_ref(), instance_program),
    ] {
        if let (Some(script), Some(program)) = (script, program) {
            let mut tester = Tester {
                state: &mut state,
                base: script.content_range.start,
            };
            tester.visit_program(program);
        }
    }
    if let Some(fragment) = fragment {
        let mut template = TemplateTester {
            source,
            ts: crate::rules::typescript_features::compiler_parses_as_ts(source),
            allocator: Allocator::default(),
            state: &mut state,
        };
        walk_with_visitor(fragment, source, &mut template);
    }
    Some(state.warned)
}

fn is_bidi(c: char) -> bool {
    matches!(c as u32, 0x202A..=0x202E | 0x2066..=0x2069)
}

#[derive(Default)]
struct State {
    /// The regex's `lastIndex`, in UTF-16 code units.
    last_index: usize,
    warned: HashSet<u32>,
}

impl State {
    /// One `test` call on `value` for the node starting at `start`.
    fn test(&mut self, value: &str, start: u32) {
        let mut offset = 0usize;
        let mut chars = value.chars().peekable();
        let mut found = false;
        while let Some(c) = chars.next() {
            if offset >= self.last_index && is_bidi(c) {
                // Bidi characters are one UTF-16 unit each; the match
                // runs to the end of the consecutive group.
                offset += 1;
                while chars.peek().is_some_and(|c| is_bidi(*c)) {
                    chars.next();
                    offset += 1;
                }
                found = true;
                break;
            }
            offset += c.len_utf16();
        }
        if found {
            self.last_index = offset;
            self.warned.insert(start);
        } else {
            self.last_index = 0;
        }
    }

    fn text(&mut self) {
        self.last_index = 0;
    }
}

struct Tester<'s> {
    state: &'s mut State,
    base: u32,
}

impl<'a> Visit<'a> for Tester<'_> {
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.state.test(&it.value, self.base + it.span.start);
    }

    fn visit_template_literal(&mut self, it: &TemplateLiteral<'a>) {
        for expr in &it.expressions {
            self.visit_expression(expr);
        }
        for quasi in &it.quasis {
            let cooked = quasi.value.cooked.as_deref().unwrap_or_default();
            self.state.test(cooked, self.base + quasi.span.start);
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
    ts: bool,
    allocator: Allocator,
    state: &'s mut State,
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
                state: self.state,
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
            state: self.state,
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
                AttrValuePart::Text { .. } => self.state.text(),
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

    fn visit_text(&mut self, _range: Range) {
        self.state.text();
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
