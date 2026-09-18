//! Compiler positions inside a script a preprocessor transpiled.
//!
//! svelte-check compiles the preprocessed component and maps each
//! diagnostic back through the preprocessor's source map. TypeScript's
//! transpile maps every emitted token from its start, so a position
//! that is a node's start comes back unchanged, while a position in
//! the middle of a token comes back at the token's start. Text
//! TypeScript deletes (type syntax, declarations that only exist for
//! the type checker, and the comments attached to those) is not in the
//! compiled code at all.
//!
//! Only one compiler diagnostic is placed by searching the source text
//! rather than at a node: `slot_snippet_conflict` without a `<slot>`
//! points at the first `$$slot` in the component. [`first_dollar_slot`]
//! finds the occurrence the compiler sees and where svelte-check
//! reports it.

use oxc_allocator::Vec as ArenaVec;
use oxc_ast::ast::{
    BigIntLiteral, BindingIdentifier, BlockStatement, ClassBody, ClassElement, Declaration,
    ExportSpecifier, FunctionBody, FunctionType, IdentifierName, IdentifierReference,
    ImportOrExportKind, ImportSpecifier, MethodDefinitionType, NumericLiteral, Program,
    PropertyDefinitionType, RegExpLiteral, Statement, StaticBlock, StringLiteral, SwitchCase,
    TSAsExpression, TSClassImplements, TSIndexSignature, TSModuleBlock, TSNamespaceDeclarationBody,
    TSSatisfiesExpression, TSThisParameter, TSTypeAnnotation, TSTypeParameterDeclaration,
    TSTypeParameterInstantiation, TemplateElement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_span::{GetSpan, Span};
use svn_parser::Document;

use crate::rules::typescript_features::script_is_transpiled;

/// Where svelte-check reports the first `$$slot` the compiler finds in
/// the preprocessed component (`preprocess_ts`: the project's
/// preprocessors transpile TypeScript scripts).
pub(crate) fn first_dollar_slot(
    doc: &Document<'_>,
    source: &str,
    module_program: Option<&Program<'_>>,
    instance_program: Option<&Program<'_>>,
    preprocess_ts: bool,
) -> u32 {
    let mut maps = Vec::new();
    for (script, program) in [
        (doc.module_script.as_ref(), module_program),
        (doc.instance_script.as_ref(), instance_program),
    ] {
        if let (Some(script), Some(program)) = (script, program)
            && script_is_transpiled(script, preprocess_ts)
        {
            maps.push(TranspiledScript::new(script.content_range.start, program));
        }
    }
    for (at, _) in source.match_indices("$$slot") {
        let at = at as u32;
        match maps.iter().find(|m| m.contains(at)) {
            None => return at,
            Some(map) => {
                if let Some(reported) = map.reported_position(at) {
                    return reported;
                }
            }
        }
    }
    // A `$$slots` reference always leaves one occurrence in code.
    0
}

/// Where svelte-check reports a compiler position `at` (absolute) inside
/// a transpiled script starting at `base`: the start of the token it
/// falls in. `None` when the transpile deletes the text there.
pub(crate) fn transpiled_position(base: u32, program: &Program<'_>, at: u32) -> Option<u32> {
    let map = TranspiledScript::new(base, program);
    if !map.contains(at) {
        return Some(at);
    }
    map.reported_position(at)
}

/// What TypeScript's transpile keeps of one script, in script-relative
/// offsets.
struct TranspiledScript {
    base: u32,
    len: u32,
    /// Text the transpile deletes.
    removed: Vec<Span>,
    /// Comments, and whether the transpile keeps each.
    comments: Vec<(Span, bool)>,
    /// Tokens with the offset their text maps from.
    tokens: Vec<(Span, u32)>,
}

impl TranspiledScript {
    fn new(base: u32, program: &Program<'_>) -> Self {
        let mut collector = Collector {
            text: program.source_text,
            comments: program.comments.iter().map(|c| c.span).collect(),
            dropped_comments: Vec::new(),
            removed: Vec::new(),
            tokens: Vec::new(),
            list_starts: vec![0],
        };
        collector.visit_program(program);
        let Collector {
            comments,
            dropped_comments,
            removed,
            tokens,
            ..
        } = collector;
        let comments = comments
            .into_iter()
            .map(|c| {
                let kept = !dropped_comments.contains(&c) && !removed.iter().any(|r| covers(*r, c));
                (c, kept)
            })
            .collect();
        Self {
            base,
            len: program.source_text.len() as u32,
            removed,
            comments,
            tokens,
        }
    }

    fn contains(&self, at: u32) -> bool {
        at >= self.base && at < self.base + self.len
    }

    /// Where an occurrence at `at` is reported, or `None` when the
    /// transpile deletes it.
    fn reported_position(&self, at: u32) -> Option<u32> {
        let rel = at - self.base;
        if self.removed.iter().any(|r| r.start <= rel && rel < r.end) {
            return None;
        }
        if let Some((span, kept)) = self
            .comments
            .iter()
            .find(|(c, _)| c.start <= rel && rel < c.end)
        {
            return kept.then_some(self.base + span.start);
        }
        let mapped = self
            .tokens
            .iter()
            .find(|(t, _)| t.start <= rel && rel < t.end)
            .map_or(rel, |(_, from)| *from);
        Some(self.base + mapped)
    }
}

fn covers(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

struct Collector<'t> {
    text: &'t str,
    comments: Vec<Span>,
    dropped_comments: Vec<Span>,
    removed: Vec<Span>,
    tokens: Vec<(Span, u32)>,
    /// Where the innermost enclosing statement or member list starts.
    list_starts: Vec<u32>,
}

impl Collector<'_> {
    fn same_line(&self, from: u32, to: u32) -> bool {
        self.text
            .get(from as usize..to as usize)
            .is_some_and(|between| !between.contains('\n'))
    }

    /// A deleted statement or class member (`span`), with the end of the
    /// sibling before it: its leading comments go with it, and so do
    /// comments after it on its last line. A comment on the previous
    /// sibling's line belongs to that sibling.
    fn remove_with_comments(&mut self, span: Span, previous_end: Option<u32>) {
        let list_start = self.list_starts.last().copied().unwrap_or(0);
        let from = previous_end.unwrap_or(list_start);
        let dropped: Vec<Span> = self
            .comments
            .iter()
            .copied()
            .filter(|c| {
                let leading = c.start >= from
                    && c.end <= span.start
                    && !previous_end.is_some_and(|p| self.same_line(p, c.start));
                let trailing = c.start >= span.end && self.same_line(span.end, c.start);
                leading || trailing
            })
            .collect();
        self.dropped_comments.extend(dropped);
        self.removed.push(span);
    }

    fn with_list<F: FnOnce(&mut Self)>(&mut self, start: u32, f: F) {
        self.list_starts.push(start);
        f(self);
        self.list_starts.pop();
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_statements(&mut self, it: &ArenaVec<'a, Statement<'a>>) {
        let mut previous_end = None;
        for stmt in it {
            if statement_removed(stmt) {
                self.remove_with_comments(stmt.span(), previous_end);
            } else {
                self.visit_statement(stmt);
            }
            previous_end = Some(stmt.span().end);
        }
    }

    fn visit_class_elements(&mut self, it: &ArenaVec<'a, ClassElement<'a>>) {
        let mut previous_end = None;
        for element in it {
            if member_removed(element) {
                self.remove_with_comments(element.span(), previous_end);
            } else {
                self.visit_class_element(element);
            }
            previous_end = Some(element.span().end);
        }
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        self.with_list(it.span.start + 1, |this| {
            walk::walk_block_statement(this, it)
        });
    }

    fn visit_function_body(&mut self, it: &FunctionBody<'a>) {
        self.with_list(it.span.start + 1, |this| walk::walk_function_body(this, it));
    }

    fn visit_ts_module_block(&mut self, it: &TSModuleBlock<'a>) {
        self.with_list(it.span.start + 1, |this| {
            walk::walk_ts_module_block(this, it)
        });
    }

    fn visit_class_body(&mut self, it: &ClassBody<'a>) {
        self.with_list(it.span.start + 1, |this| walk::walk_class_body(this, it));
    }

    fn visit_static_block(&mut self, it: &StaticBlock<'a>) {
        self.with_list(it.span.start, |this| walk::walk_static_block(this, it));
    }

    fn visit_switch_case(&mut self, it: &SwitchCase<'a>) {
        self.with_list(it.span.start, |this| walk::walk_switch_case(this, it));
    }

    // Type syntax.
    fn visit_ts_type_annotation(&mut self, it: &TSTypeAnnotation<'a>) {
        self.removed.push(it.span);
    }

    fn visit_ts_type_parameter_declaration(&mut self, it: &TSTypeParameterDeclaration<'a>) {
        self.removed.push(it.span);
    }

    fn visit_ts_type_parameter_instantiation(&mut self, it: &TSTypeParameterInstantiation<'a>) {
        self.removed.push(it.span);
    }

    fn visit_ts_this_parameter(&mut self, it: &TSThisParameter<'a>) {
        self.removed.push(it.span);
    }

    fn visit_ts_index_signature(&mut self, it: &TSIndexSignature<'a>) {
        self.removed.push(it.span);
    }

    fn visit_ts_class_implements_list(&mut self, it: &ArenaVec<'a, TSClassImplements<'a>>) {
        if let (Some(first), Some(last)) = (it.first(), it.last()) {
            self.removed
                .push(Span::new(first.span.start, last.span.end));
        }
    }

    fn visit_ts_as_expression(&mut self, it: &TSAsExpression<'a>) {
        self.visit_expression(&it.expression);
        self.removed
            .push(Span::new(it.expression.span().end, it.span.end));
    }

    fn visit_ts_satisfies_expression(&mut self, it: &TSSatisfiesExpression<'a>) {
        self.visit_expression(&it.expression);
        self.removed
            .push(Span::new(it.expression.span().end, it.span.end));
    }

    fn visit_import_specifier(&mut self, it: &ImportSpecifier<'a>) {
        if it.import_kind == ImportOrExportKind::Type {
            self.removed.push(it.span);
        } else {
            walk::walk_import_specifier(self, it);
        }
    }

    fn visit_export_specifier(&mut self, it: &ExportSpecifier<'a>) {
        if it.export_kind == ImportOrExportKind::Type {
            self.removed.push(it.span);
        } else {
            walk::walk_export_specifier(self, it);
        }
    }

    // Tokens and the offset their text maps from.
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.tokens.push((it.span, it.span.start));
    }

    fn visit_numeric_literal(&mut self, it: &NumericLiteral<'a>) {
        self.tokens.push((it.span, it.span.start));
    }

    fn visit_big_int_literal(&mut self, it: &BigIntLiteral<'a>) {
        self.tokens.push((it.span, it.span.start));
    }

    fn visit_reg_exp_literal(&mut self, it: &RegExpLiteral<'a>) {
        self.tokens.push((it.span, it.span.start));
    }

    fn visit_template_element(&mut self, it: &TemplateElement<'a>) {
        // A template chunk maps from the delimiter before it.
        self.tokens.push((it.span, it.span.start.saturating_sub(1)));
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        self.tokens.push((it.span, it.span.start));
    }

    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        self.tokens.push((it.span, it.span.start));
    }

    fn visit_identifier_name(&mut self, it: &IdentifierName<'a>) {
        self.tokens.push((it.span, it.span.start));
    }
}

/// A statement TypeScript's transpile deletes outright.
fn statement_removed(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::ImportDeclaration(d) => d.import_kind == ImportOrExportKind::Type,
        Statement::ExportNamedDeclaration(d) => d.export_kind == ImportOrExportKind::Type,
        Statement::ExportFromDeclaration(d) => d.export_kind == ImportOrExportKind::Type,
        Statement::ExportAllDeclaration(d) => d.export_kind == ImportOrExportKind::Type,
        Statement::ExportDeclaration(d) => declaration_removed(&d.declaration),
        Statement::VariableDeclaration(d) => d.declare,
        Statement::FunctionDeclaration(f) => f.r#type == FunctionType::TSDeclareFunction,
        Statement::ClassDeclaration(c) => c.declare,
        Statement::TSTypeAliasDeclaration(_)
        | Statement::TSInterfaceDeclaration(_)
        | Statement::TSGlobalDeclaration(_) => true,
        Statement::TSEnumDeclaration(e) => e.declare,
        Statement::TSExternalModuleDeclaration(_) => true,
        Statement::TSNamespaceDeclaration(n) => namespace_removed(n),
        Statement::TSImportEqualsDeclaration(d) => d.import_kind == ImportOrExportKind::Type,
        Statement::ExportDefaultDeclaration(_)
        | Statement::TSExportAssignment(_)
        | Statement::TSNamespaceExportDeclaration(_)
        | Statement::BlockStatement(_)
        | Statement::BreakStatement(_)
        | Statement::ContinueStatement(_)
        | Statement::DebuggerStatement(_)
        | Statement::DoWhileStatement(_)
        | Statement::EmptyStatement(_)
        | Statement::ExpressionStatement(_)
        | Statement::ForInStatement(_)
        | Statement::ForOfStatement(_)
        | Statement::ForStatement(_)
        | Statement::IfStatement(_)
        | Statement::LabeledStatement(_)
        | Statement::ReturnStatement(_)
        | Statement::SwitchStatement(_)
        | Statement::ThrowStatement(_)
        | Statement::TryStatement(_)
        | Statement::WhileStatement(_)
        | Statement::WithStatement(_) => false,
    }
}

/// [`statement_removed`] for the declaration of an `export` statement.
fn declaration_removed(decl: &Declaration<'_>) -> bool {
    match decl {
        Declaration::VariableDeclaration(d) => d.declare,
        Declaration::FunctionDeclaration(f) => f.r#type == FunctionType::TSDeclareFunction,
        Declaration::ClassDeclaration(c) => c.declare,
        Declaration::TSTypeAliasDeclaration(_)
        | Declaration::TSInterfaceDeclaration(_)
        | Declaration::TSGlobalDeclaration(_)
        | Declaration::TSExternalModuleDeclaration(_) => true,
        Declaration::TSEnumDeclaration(e) => e.declare,
        Declaration::TSNamespaceDeclaration(n) => namespace_removed(n),
        Declaration::TSImportEqualsDeclaration(d) => d.import_kind == ImportOrExportKind::Type,
    }
}

/// A namespace is deleted when it is declared or holds nothing but
/// deleted statements.
pub(crate) fn namespace_removed(n: &oxc_ast::ast::TSNamespaceDeclaration<'_>) -> bool {
    n.declare
        || match &n.body {
            TSNamespaceDeclarationBody::TSModuleBlock(block) => {
                block.body.iter().all(statement_removed)
            }
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => namespace_removed(inner),
        }
}

/// A class member TypeScript's transpile deletes outright.
fn member_removed(element: &ClassElement<'_>) -> bool {
    match element {
        ClassElement::PropertyDefinition(p) => {
            p.declare || p.r#type == PropertyDefinitionType::TSAbstractPropertyDefinition
        }
        ClassElement::MethodDefinition(m) => {
            m.r#type == MethodDefinitionType::TSAbstractMethodDefinition || m.value.body.is_none()
        }
        ClassElement::TSIndexSignature(_) => true,
        ClassElement::StaticBlock(_) | ClassElement::AccessorProperty(_) => false,
    }
}
