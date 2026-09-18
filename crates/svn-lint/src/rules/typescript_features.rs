//! `typescript_invalid_feature`: TypeScript constructs the compiler
//! refuses to strip.
//!
//! When a component is TypeScript, the compiler removes type-only
//! syntax from its template, instance script and module script (in
//! that order) before analysing anything (`remove_typescript_nodes`).
//! The removal throws on constructs that would need a real
//! transformation: enums, decorators, `accessor` fields, `readonly` or
//! accessibility modifiers on constructor parameters, and namespaces
//! that hold anything but types. The first one found fails the whole
//! component, ahead of every analysis diagnostic.
//!
//! svelte-check compiles the component after running the project's
//! preprocessors. A preprocessor that transpiles `<script lang="ts">`
//! (the language server's fallback when there is no Svelte config,
//! `vitePreprocess({ script: true })`, `svelte-preprocess`) leaves only
//! what TypeScript's own ES-next output keeps: decorators and
//! `accessor` fields. Declarations TypeScript drops (`declare`
//! members, overload signatures, abstract methods, declared
//! namespaces) are dropped there too; enums and namespaces become
//! plain code the removal walks through.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    AccessorProperty, ArrowFunctionExpression, Class, Declaration, Decorator, ExportAllDeclaration,
    ExportDefaultDeclaration, ExportFromDeclaration, ExportNamedDeclaration, FormalParameter,
    Function, FunctionType, ImportDeclaration, ImportDeclarationSpecifier, ImportOrExportKind,
    MethodDefinition, MethodDefinitionType, Program, Statement, TSEnumDeclaration,
    TSExternalModuleDeclaration, TSGlobalDeclaration, TSInterfaceDeclaration, TSModuleBlock,
    TSNamespaceDeclaration, TSNamespaceDeclarationBody, TSTypeAliasDeclaration,
    VariableDeclaration,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};
use oxc_syntax::scope::ScopeFlags;
use smol_str::SmolStr;
use svn_analyze::template_scope::{TemplateScopeVisitor, walk_with_visitor};
use svn_core::Range;
use svn_parser::ast::{AttrValuePart, Attribute, DirectiveValue, Fragment};
use svn_parser::{Component, Element, SvelteElement};

/// What the removal pass does with a script or template expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Finding {
    /// It throws `typescript_invalid_feature` for `feature` at `range`
    /// (absolute source offsets).
    Invalid { feature: &'static str, range: Range },
    /// It crashes: a dotted namespace (`namespace A.B {}`) has another
    /// namespace as its body, which the removal cannot iterate. The
    /// language server reports the resulting exception without a code.
    Crash,
}

/// The exception the removal throws on a dotted namespace. The
/// visitor reads `node.body.body.map(…)`; the body is the inner
/// namespace, which has no `body` array, and every published build of
/// the compiler names that parameter `e`, so Node reports this.
pub(crate) const DOTTED_NAMESPACE_EXCEPTION: &str = "e.body.body.map is not a function";

const DECORATORS: &str = "decorators (related TSC proposal is not stage 4 yet)";
const ACCESSOR_FIELDS: &str = "accessor fields (related TSC proposal is not stage 4 yet)";
const ENUMS: &str = "enums";
const PARAMETER_MODIFIERS: &str = "accessibility modifiers on constructor parameters";
const NAMESPACES: &str = "namespaces with non-type nodes";

/// The compiler's TypeScript switch: the first `<script>` tag carrying
/// a `lang=` attribute decides, and only the exact value `ts` counts.
/// A port of `regex_lang_attribute` in `phases/1-parse/index.js`
/// (comments are matched and skipped; a tag whose `lang=` value does
/// not fit the pattern is passed over).
pub(crate) fn compiler_parses_as_ts(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let Some(rel) = source[at..].find('<') else {
            return false;
        };
        let start = at + rel;
        let rest = &source[start..];
        if let Some(body) = rest.strip_prefix("<!--")
            && let Some(end) = body.find("-->")
        {
            at = start + 4 + end + 3;
            continue;
        }
        if let Some(after) = rest.strip_prefix("<script")
            && after.starts_with(|c: char| c.is_ascii_whitespace())
            && let Some(value) = script_tag_lang(after)
        {
            return value == "ts";
        }
        at = start + 1;
    }
    false
}

/// The `lang=` value the compiler's pattern reads from a `<script`
/// tag's attribute text: the last `lang=` before the tag's first `>`
/// whose value is `"v"`, `'v'` or a bare `v` (no quotes, spaces or
/// `>` inside) followed by the end of the tag.
fn script_tag_lang(after: &str) -> Option<&str> {
    let tag = &after[..after.find('>')?];
    let mut search_end = tag.len();
    while let Some(pos) = tag[..search_end].rfind("lang=") {
        let value = &tag[pos + "lang=".len()..];
        let (quote, body) = match value.as_bytes().first() {
            Some(b'"') => (Some('"'), &value[1..]),
            Some(b'\'') => (Some('\''), &value[1..]),
            _ => (None, value),
        };
        let len = body.find(['"', '\'', ' ', '>']).unwrap_or(body.len());
        let closes = match quote {
            Some(q) => body[len..].starts_with(q),
            None => true,
        };
        if len > 0 && closes {
            return Some(&body[..len]);
        }
        search_end = pos;
    }
    None
}

/// Whether the project's preprocessors (`preprocess_ts`: they
/// transpile TypeScript scripts at all) turn this script into
/// JavaScript before the compiler sees it. A preprocessor reads the
/// tag's attributes as an object, so the last `lang` wins, and only the
/// exact value `ts` is handled.
pub(crate) fn script_is_transpiled(
    script: &svn_parser::ScriptSection<'_>,
    preprocess_ts: bool,
) -> bool {
    preprocess_ts
        && script
            .attrs
            .iter()
            .rev()
            .find(|a| a.name == "lang")
            .is_some_and(|a| a.value.as_deref() == Some("ts"))
}

/// The first finding in a script (or a template expression parsed as
/// a program), with spans offset by `base`. `transpiled` says a
/// preprocessor turned the script into JavaScript first.
pub(crate) fn first_finding(program: &Program<'_>, base: u32, transpiled: bool) -> Option<Finding> {
    let mut checker = Checker {
        text: program.source_text,
        base,
        transpiled,
        constructor_params: Vec::new(),
        found: None,
    };
    checker.visit_program(program);
    checker.found
}

struct Checker<'t> {
    /// The parsed text, which spans index.
    text: &'t str,
    base: u32,
    transpiled: bool,
    /// For each enclosing function, whether its parameters are a class
    /// constructor's.
    constructor_params: Vec<bool>,
    found: Option<Finding>,
}

impl Checker<'_> {
    fn invalid(&mut self, feature: &'static str, span: Span) {
        if self.found.is_none() {
            self.found = Some(Finding::Invalid {
                feature,
                range: Range::new(self.base + span.start, self.base + span.end),
            });
        }
    }

    /// A namespace or module declaration's body. The removal visits
    /// every statement (throwing on the first invalid one), then fails
    /// the declaration itself when any statement survived as code.
    fn namespace_body(&mut self, block: Option<&TSModuleBlock<'_>>, declare: bool, span: Span) {
        if self.transpiled {
            // TypeScript drops declared namespaces and rewrites the
            // others into plain code the removal walks through.
            if !declare && let Some(block) = block {
                self.visit_statements(&block.body);
            }
            return;
        }
        let Some(block) = block else {
            return;
        };
        let mut keeps_code = false;
        for stmt in &block.body {
            self.visit_statement(stmt);
            if self.found.is_some() {
                return;
            }
            keeps_code |= !removed(stmt);
        }
        if keeps_code {
            self.invalid(NAMESPACES, span);
        }
    }
}

impl<'a> Visit<'a> for Checker<'_> {
    fn visit_statement(&mut self, it: &Statement<'a>) {
        if self.found.is_none() {
            walk::walk_statement(self, it);
        }
    }

    fn visit_decorator(&mut self, it: &Decorator<'a>) {
        self.invalid(DECORATORS, it.span);
    }

    fn visit_accessor_property(&mut self, it: &AccessorProperty<'a>) {
        self.invalid(ACCESSOR_FIELDS, it.span);
    }

    fn visit_ts_enum_declaration(&mut self, it: &TSEnumDeclaration<'a>) {
        if self.transpiled {
            if !it.declare {
                walk::walk_ts_enum_declaration(self, it);
            }
        } else {
            self.invalid(ENUMS, it.span);
        }
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        let is_property = it.accessibility.is_some() || it.readonly || it.r#override;
        if !is_property {
            walk::walk_formal_parameter(self, it);
            return;
        }
        if self.transpiled {
            // TypeScript turns the property into an assignment but
            // keeps the parameter's decorators.
            walk::walk_formal_parameter(self, it);
            return;
        }
        // The removal checks a parameter property's modifiers, then
        // visits its binding only, not its decorators. The property
        // node starts at its first modifier, after any decorator.
        if (it.accessibility.is_some() || it.readonly)
            && self.constructor_params.last() == Some(&true)
        {
            let start = it
                .decorators
                .last()
                .map_or(it.span.start, |d| skip_whitespace(self.text, d.span.end));
            self.invalid(PARAMETER_MODIFIERS, Span::new(start, it.span.end));
            return;
        }
        self.visit_binding_pattern(&it.pattern);
        if let Some(init) = &it.initializer {
            self.visit_expression(init);
        }
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        if it.r#type == FunctionType::TSDeclareFunction {
            return;
        }
        self.constructor_params
            .push(flags.contains(ScopeFlags::Constructor));
        walk::walk_function(self, it, flags);
        self.constructor_params.pop();
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        self.constructor_params.push(false);
        walk::walk_arrow_function_expression(self, it);
        self.constructor_params.pop();
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        if !it.declare {
            walk::walk_class(self, it);
        }
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        if it.r#type != MethodDefinitionType::TSAbstractMethodDefinition {
            walk::walk_method_definition(self, it);
        }
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if !it.declare {
            walk::walk_variable_declaration(self, it);
        }
    }

    fn visit_ts_namespace_declaration(&mut self, it: &TSNamespaceDeclaration<'a>) {
        match &it.body {
            TSNamespaceDeclarationBody::TSModuleBlock(block) => {
                self.namespace_body(Some(block), it.declare, it.span);
            }
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => {
                if !self.transpiled {
                    if self.found.is_none() {
                        self.found = Some(Finding::Crash);
                    }
                } else if !it.declare {
                    self.visit_ts_namespace_declaration(inner);
                }
            }
        }
    }

    fn visit_ts_external_module_declaration(&mut self, it: &TSExternalModuleDeclaration<'a>) {
        self.namespace_body(it.body.as_deref(), true, it.span);
    }

    fn visit_ts_global_declaration(&mut self, it: &TSGlobalDeclaration<'a>) {
        self.namespace_body(Some(&it.body), true, it.span);
    }

    // Left as written by the removal, so never searched.
    fn visit_import_declaration(&mut self, _it: &ImportDeclaration<'a>) {}
    fn visit_export_default_declaration(&mut self, _it: &ExportDefaultDeclaration<'a>) {}
    fn visit_export_all_declaration(&mut self, _it: &ExportAllDeclaration<'a>) {}
    fn visit_export_named_declaration(&mut self, _it: &ExportNamedDeclaration<'a>) {}
    fn visit_export_from_declaration(&mut self, _it: &ExportFromDeclaration<'a>) {}
    fn visit_ts_interface_declaration(&mut self, _it: &TSInterfaceDeclaration<'a>) {}
    fn visit_ts_type_alias_declaration(&mut self, _it: &TSTypeAliasDeclaration<'a>) {}
}

/// The offset of the first non-whitespace character at or after `at`.
fn skip_whitespace(text: &str, at: u32) -> u32 {
    let rest = text.get(at as usize..).unwrap_or_default();
    at + (rest.len() - rest.trim_start().len()) as u32
}

/// Whether the removal replaces a namespace-body statement with its
/// shared empty node (anything else counts as code, a stray `;`
/// included).
fn removed(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::ImportDeclaration(d) => {
            d.import_kind == ImportOrExportKind::Type
                || d.specifiers.as_ref().is_some_and(|specs| {
                    !specs.is_empty()
                        && specs.iter().all(|s| match s {
                            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                                s.import_kind == ImportOrExportKind::Type
                            }
                            ImportDeclarationSpecifier::ImportDefaultSpecifier(_)
                            | ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => false,
                        })
                })
        }
        Statement::ExportNamedDeclaration(d) => {
            d.export_kind == ImportOrExportKind::Type
                || d.specifiers
                    .iter()
                    .all(|s| s.export_kind == ImportOrExportKind::Type)
        }
        Statement::ExportFromDeclaration(d) => {
            d.export_kind == ImportOrExportKind::Type
                || d.specifiers
                    .iter()
                    .all(|s| s.export_kind == ImportOrExportKind::Type)
        }
        Statement::ExportAllDeclaration(d) => d.export_kind == ImportOrExportKind::Type,
        Statement::ExportDeclaration(d) => declaration_removed(&d.declaration),
        Statement::ExportDefaultDeclaration(_)
        | Statement::TSExportAssignment(_)
        | Statement::TSNamespaceExportDeclaration(_) => false,
        Statement::VariableDeclaration(d) => d.declare,
        Statement::FunctionDeclaration(f) => f.r#type == FunctionType::TSDeclareFunction,
        Statement::ClassDeclaration(c) => c.declare,
        Statement::TSTypeAliasDeclaration(_)
        | Statement::TSInterfaceDeclaration(_)
        | Statement::TSExternalModuleDeclaration(_)
        | Statement::TSNamespaceDeclaration(_)
        | Statement::TSGlobalDeclaration(_) => true,
        // An enum throws before this is asked.
        Statement::TSEnumDeclaration(_) | Statement::TSImportEqualsDeclaration(_) => false,
        Statement::BlockStatement(_)
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

/// [`removed`] for the declaration of an `export` statement.
fn declaration_removed(decl: &Declaration<'_>) -> bool {
    match decl {
        Declaration::VariableDeclaration(d) => d.declare,
        Declaration::FunctionDeclaration(f) => f.r#type == FunctionType::TSDeclareFunction,
        Declaration::ClassDeclaration(c) => c.declare,
        Declaration::TSTypeAliasDeclaration(_)
        | Declaration::TSInterfaceDeclaration(_)
        | Declaration::TSExternalModuleDeclaration(_)
        | Declaration::TSNamespaceDeclaration(_)
        | Declaration::TSGlobalDeclaration(_) => true,
        Declaration::TSEnumDeclaration(_) | Declaration::TSImportEqualsDeclaration(_) => false,
    }
}

/// The first finding in the template's expressions. The template is
/// never preprocessed, and only a class can hold a finding there, so
/// expressions without the text `class` are not parsed.
pub(crate) fn first_template_finding(fragment: &Fragment, source: &str) -> Option<Finding> {
    let mut finder = TemplateFinder {
        source,
        allocator: Allocator::default(),
        found: None,
    };
    walk_with_visitor(fragment, source, &mut finder);
    finder.found
}

struct TemplateFinder<'s> {
    source: &'s str,
    allocator: Allocator,
    found: Option<Finding>,
}

impl TemplateFinder<'_> {
    fn check(&mut self, range: Range, declaration: bool) {
        if self.found.is_some() {
            return;
        }
        let Some(text) = self.source.get(range.start as usize..range.end as usize) else {
            return;
        };
        if !text.contains("class") {
            return;
        }
        let source_type = SourceType::default()
            .with_module(true)
            .with_typescript(true);
        self.allocator.reset();
        if declaration {
            // `{@const NAME = EXPR}`: read the body as a declaration,
            // shifting spans back over the `let ` prefix.
            let wrapped = format!("let {text}");
            let parsed = Parser::new(&self.allocator, &wrapped, source_type).parse();
            self.found = first_finding(&parsed.program, range.start.wrapping_sub(4), false);
        } else if let Ok(expr) = Parser::new(&self.allocator, text, source_type).parse_expression()
        {
            let mut checker = Checker {
                text,
                base: range.start,
                transpiled: false,
                constructor_params: Vec::new(),
                found: None,
            };
            checker.visit_expression(&expr);
            self.found = checker.found;
        }
    }

    fn check_attributes(&mut self, attributes: &[Attribute]) {
        for attr in attributes {
            match attr {
                Attribute::Plain(p) => {
                    if let Some(v) = &p.value {
                        self.check_parts(&v.parts);
                    }
                }
                Attribute::Expression(e) => self.check(e.expression_range, false),
                Attribute::Spread(s) => self.check(s.expression_range, false),
                Attribute::Directive(d) => match &d.value {
                    Some(DirectiveValue::Expression {
                        expression_range, ..
                    }) => self.check(*expression_range, false),
                    Some(DirectiveValue::Quoted(v)) => self.check_parts(&v.parts),
                    Some(DirectiveValue::BindPair { .. }) | None => {}
                },
                Attribute::Shorthand(_) | Attribute::Comment(_) => {}
            }
        }
    }

    fn check_parts(&mut self, parts: &[AttrValuePart]) {
        for part in parts {
            if let AttrValuePart::Expression {
                expression_range, ..
            } = part
            {
                self.check(*expression_range, false);
            }
        }
    }
}

impl TemplateScopeVisitor for TemplateFinder<'_> {
    fn visit_expr(&mut self, range: Range) {
        self.check(range, false);
    }

    fn visit_at_const(&mut self, _bound_names: &[SmolStr], expr_range: Range) {
        self.check(expr_range, true);
    }

    fn visit_element(&mut self, element: &Element) {
        self.check_attributes(&element.attributes);
    }

    fn visit_component(&mut self, component: &Component) {
        self.check_attributes(&component.attributes);
    }

    fn visit_svelte_element(&mut self, element: &SvelteElement) {
        self.check_attributes(&element.attributes);
    }
}

#[cfg(test)]
mod tests {
    use super::compiler_parses_as_ts;

    #[test]
    fn first_script_with_a_lang_decides() {
        assert!(compiler_parses_as_ts("<script lang=\"ts\"></script>"));
        assert!(compiler_parses_as_ts(
            "<script module>x</script><script lang='ts'></script>"
        ));
        assert!(compiler_parses_as_ts(
            "<script generics=\"T\" lang=ts></script>"
        ));
        assert!(!compiler_parses_as_ts(
            "<script lang=\"typescript\"></script>"
        ));
        assert!(!compiler_parses_as_ts(
            "<script module lang=\"js\"></script><script lang=\"ts\"></script>"
        ));
        assert!(!compiler_parses_as_ts("<!-- <script lang=\"ts\"> -->"));
        assert!(!compiler_parses_as_ts("<script></script>"));
        // A value the pattern cannot read is passed over.
        assert!(compiler_parses_as_ts(
            "<script lang=\"js \"></script><script lang=\"ts\"></script>"
        ));
    }
}
