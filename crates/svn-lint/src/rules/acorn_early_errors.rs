//! Errors the compiler's script parser (acorn, extended by
//! `@sveltejs/acorn-typescript` for TypeScript components) raises on
//! input oxc's parser accepts.
//!
//! oxc leaves most of the language's "early errors" to its semantic
//! pass, which we do not run: redeclared bindings, duplicate parameter
//! names, undefined labels, strict-mode restrictions, misplaced `super`,
//! undeclared private names and so on. acorn raises them while parsing,
//! so a script that has one never reaches the compiler's analysis. This
//! module walks the oxc AST and replays those checks with acorn's
//! messages and positions.
//!
//! A script is parsed as a module, so it is always strict code. acorn's
//! scopes differ from the language's in one visible way that is kept
//! here: a function's parameters and its body's top-level declarations
//! share one scope.
//!
//! It also replays what the parsers do with syntax they do not
//! support, where oxc is more lenient:
//!
//! - plain acorn (a JavaScript component) has no TypeScript: oxc still
//!   reads `as` / `satisfies`, parameter and member modifiers,
//!   decorators and `accessor` fields there, and acorn stops at them;
//! - neither parser version supports `using` declarations or import
//!   phases (`import defer`).
//!
//! It runs only on a script a preprocessor transpiled: one the compiler
//! reads as written is read by the acorn port itself
//! (`js_parse_error`). So a TypeScript script here is checked as what
//! TypeScript's transpile keeps of it: type-only declarations and
//! imports are gone, and an enum or namespace is a `var`.

use oxc_ast::ast::{
    ArrowFunctionBody, ArrowFunctionExpression, AssignmentTargetPropertyIdentifier,
    AwaitExpression, BindingIdentifier, BindingPattern, BlockStatement, BreakStatement,
    CallExpression, CatchClause, ChainElement, Class, ClassElement, ContinueStatement, Decorator,
    Directive, DoWhileStatement, Expression, ForInStatement, ForOfStatement, ForStatement,
    ForStatementInit, ForStatementLeft, FormalParameter, FormalParameters, Function, FunctionBody,
    FunctionType, IdentifierReference, IfStatement, ImportDeclaration, ImportDeclarationSpecifier,
    ImportOrExportKind, LabelIdentifier, LabeledStatement, MethodDefinitionKind, NumericLiteral,
    ObjectExpression, ObjectPropertyKind, PrivateFieldExpression, PrivateInExpression, Program,
    PropertyKey, PropertyKind, RegExpLiteral, SimpleAssignmentTarget, Statement, StaticBlock,
    StringLiteral, Super, SwitchStatement, TSAsExpression, TSClassImplements, TSEnumDeclaration,
    TSExternalModuleDeclaration, TSGlobalDeclaration, TSImportEqualsDeclaration,
    TSInterfaceDeclaration, TSNamespaceDeclaration, TSNamespaceDeclarationBody,
    TSSatisfiesExpression, TSType, TSTypeAliasDeclaration, TSTypeAnnotation,
    TSTypeParameterDeclaration, TSTypeParameterInstantiation, TaggedTemplateExpression,
    UnaryExpression, UnaryOperator, VariableDeclaration, VariableDeclarationKind,
    VariableDeclarator, WhileStatement, WithStatement, YieldExpression,
};
use oxc_ast::ast::{ConditionalExpression, ImportExpression};
use oxc_ast_visit::{Visit, walk};
use oxc_span::{GetSpan, Span};
use oxc_syntax::scope::ScopeFlags;

/// Where a parse error is reported, relative to the script text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Position {
    /// At this offset.
    At(u32),
    /// acorn-typescript raises some modifier errors at the column of
    /// the modifier's token, passed where an offset belongs: the error
    /// lands at the file offset equal to that column.
    ColumnOf(u32),
    /// The parser failed without an offset. The compiler's error has
    /// no position, and one is shown at the start of the file.
    Nowhere,
}

/// One parse error the compiler would raise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// Where the parser is when it raises the error. The parser stops
    /// at its first error, so the candidate raised earliest wins.
    pub detect: u32,
    pub at: Position,
    /// acorn's message, without the compiler's docs link.
    pub message: String,
    /// The start of the token the position lies in, when it is not the
    /// token's start: a transpiled script maps it back from there.
    pub token_start: Option<u32>,
}

impl Candidate {
    pub(crate) fn at(offset: u32, message: impl Into<String>) -> Self {
        Self {
            detect: offset,
            at: Position::At(offset),
            message: message.into(),
            token_start: None,
        }
    }

    pub(crate) fn in_token(mut self, start: u32) -> Self {
        self.token_start = Some(start);
        self
    }
}

/// How the compiler reads the script.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mode {
    /// The component is TypeScript: acorn-typescript parses the script.
    pub ts: bool,
}

/// The earliest early error in a script. `text` is the script body the
/// program was parsed from.
pub(crate) fn first_early_error(
    program: &Program<'_>,
    text: &str,
    mode: Mode,
) -> Option<Candidate> {
    let mut checker = Checker {
        text,
        mode,
        scopes: Vec::new(),
        classes: Vec::new(),
        declaration_kind: None,
        top_level: module_level_statements(program),
        in_parameter_default: false,
        parameter_await_is_expression: false,
        best: None,
    };
    // A directive is a string literal like any other.
    for directive in &program.directives {
        checker.check_string_escapes(directive.expression.span);
    }
    checker.scopes.push(Scope {
        flags: TOP,
        context: Some(Context::default()),
        ..Scope::default()
    });
    checker.visit_statements(&program.body);
    checker.best
}

/// Where the statements start that may be an `import` or `export`: the
/// program's own, and those of a TypeScript namespace or module body,
/// where `export` marks what the namespace exposes.
fn module_level_statements(program: &Program<'_>) -> Vec<u32> {
    struct Collector(Vec<u32>);
    impl<'a> Visit<'a> for Collector {
        fn visit_ts_module_block(&mut self, it: &oxc_ast::ast::TSModuleBlock<'a>) {
            self.0.extend(it.body.iter().map(|s| s.span().start));
            walk::walk_ts_module_block(self, it);
        }
    }
    let mut collector = Collector(program.body.iter().map(|s| s.span().start).collect());
    collector.visit_program(program);
    collector.0
}

/// Scope flags, as acorn keeps them.
const TOP: u8 = 1;
const FUNCTION: u8 = 2;
const SIMPLE_CATCH: u8 = 4;
const STATIC_BLOCK: u8 = 8;
/// Scopes `var` declarations stop at.
const VAR_SCOPE: u8 = TOP | FUNCTION | STATIC_BLOCK;

#[derive(Default)]
struct Scope<'t> {
    flags: u8,
    var: Vec<&'t str>,
    lexical: Vec<&'t str>,
    functions: Vec<&'t str>,
    /// Set on the scope of a function, static block or field
    /// initializer: what code directly inside it may do.
    context: Option<Context<'t>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bind {
    Var,
    Lexical,
    SimpleCatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LabelKind {
    Loop,
    Switch,
    Other,
}

struct Label<'t> {
    name: Option<&'t str>,
    kind: LabelKind,
}

/// What the enclosing function (or class member initializer) allows.
#[derive(Default)]
struct Context<'t> {
    /// A function (or arrow) body, and whether it is `async`.
    function: bool,
    is_async: bool,
    generator: bool,
    allow_super: bool,
    allow_direct_super: bool,
    /// Inside a class field initializer (arrows included).
    field_init: bool,
    /// Directly inside a class static block.
    static_block: bool,
    labels: Vec<Label<'t>>,
}

/// Private names of one class body.
#[derive(Default)]
struct PrivateNames<'t> {
    /// Declared names, with getter/setter pairing state.
    declared: Vec<(&'t str, &'static str)>,
    /// Uses, in source order.
    used: Vec<(&'t str, u32)>,
}

struct Checker<'s, 't> {
    text: &'s str,
    mode: Mode,
    scopes: Vec<Scope<'t>>,
    classes: Vec<PrivateNames<'t>>,
    /// The kind of the variable declaration being visited.
    declaration_kind: Option<VariableDeclarationKind>,
    /// Where the program's own statements start: an `import` or
    /// `export` anywhere else is not at the top level.
    top_level: Vec<u32>,
    /// Inside the default value of a parameter of the innermost
    /// function: acorn refuses a `yield` or `await` expression there.
    in_parameter_default: bool,
    /// Whether an `await` in the innermost function's parameters is an
    /// await expression: the function is async, or it is an arrow
    /// function whose parameters acorn first read as an expression
    /// where `await` is one.
    parameter_await_is_expression: bool,
    best: Option<Candidate>,
}

const STRICT_RESERVED: &[&str] = &[
    "implements",
    "interface",
    "let",
    "package",
    "private",
    "protected",
    "public",
    "static",
    "yield",
];

/// Words acorn-typescript reads as class member modifiers.
const MEMBER_MODIFIERS: &[&str] = &[
    "declare",
    "private",
    "public",
    "protected",
    "accessor",
    "override",
    "abstract",
    "readonly",
    "static",
];

/// TypeScript member modifiers plain acorn cannot read.
const TS_ONLY_MEMBER_MODIFIERS: &[&str] = &[
    "declare",
    "private",
    "public",
    "protected",
    "override",
    "abstract",
    "readonly",
];

impl<'s, 't> Checker<'s, 't> {
    fn record(&mut self, candidate: Candidate) {
        if self
            .best
            .as_ref()
            .is_none_or(|best| candidate.detect < best.detect)
        {
            self.best = Some(candidate);
        }
    }

    /// An escaped `in` / `instanceof` starting a class member on the
    /// line after a field initializer: oxc reads it as the member's
    /// name, but acorn's tokenizer gives it the keyword's type, so the
    /// initializer takes it as a binary operator and consuming an
    /// escaped keyword fails.
    fn escaped_operator_after_field(
        &mut self,
        previous: &ClassElement<'_>,
        element: &ClassElement<'_>,
    ) {
        let ClassElement::PropertyDefinition(field) = previous else {
            return;
        };
        let Some(value) = &field.value else {
            return;
        };
        let key = match element {
            ClassElement::PropertyDefinition(p) => &p.key,
            ClassElement::MethodDefinition(m) => &m.key,
            _ => return,
        };
        let PropertyKey::StaticIdentifier(id) = key else {
            return;
        };
        let start = id.span.start;
        let written = &self.text[start as usize..id.span.end as usize];
        if !matches!(id.name.as_str(), "in" | "instanceof")
            || written == id.name.as_str()
            || element.span().start != start
            || self.next_token(value.span().end) != start
            || !self.text[value.span().end as usize..start as usize]
                .contains(['\n', '\r', '\u{2028}', '\u{2029}'])
            || !continues_with_operator(value)
        {
            return;
        }
        self.error(start, format!("Escape sequence in keyword {}", id.name));
    }

    fn error(&mut self, at: u32, message: impl Into<String>) {
        self.record(Candidate::at(at, message));
    }

    /// Nothing reported at or after `at` can win any more.
    fn settled(&self, at: u32) -> bool {
        self.best.as_ref().is_some_and(|b| b.detect <= at)
    }

    fn js(&self) -> bool {
        !self.mode.ts
    }

    fn push_scope(&mut self, flags: u8) {
        self.scopes.push(Scope {
            flags,
            ..Scope::default()
        });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn context(&self) -> &Context<'t> {
        const PROGRAM: &Context<'static> = &Context {
            function: false,
            is_async: false,
            generator: false,
            allow_super: false,
            allow_direct_super: false,
            field_init: false,
            static_block: false,
            labels: Vec::new(),
        };
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.context.as_ref())
            .unwrap_or(PROGRAM)
    }

    fn context_mut(&mut self) -> Option<&mut Context<'t>> {
        self.scopes
            .iter_mut()
            .rev()
            .find_map(|s| s.context.as_mut())
    }

    /// The first token at or after `at`.
    fn next_token(&self, at: u32) -> u32 {
        next_token_start(self.text, at)
    }

    /// acorn's `declareName`.
    fn declare(&mut self, name: &'t str, bind: Bind, at: u32) {
        let depth = self.scopes.len();
        let Some(scope) = self.scopes.last_mut() else {
            return;
        };
        let redeclared = match bind {
            Bind::Lexical => {
                let redeclared = scope.lexical.contains(&name)
                    || scope.functions.contains(&name)
                    || scope.var.contains(&name);
                scope.lexical.push(name);
                redeclared
            }
            Bind::SimpleCatch => {
                scope.lexical.push(name);
                false
            }
            Bind::Var => {
                let mut redeclared = false;
                for i in (0..depth).rev() {
                    let scope = &mut self.scopes[i];
                    let catch_param =
                        scope.flags & SIMPLE_CATCH != 0 && scope.lexical.first() == Some(&name);
                    if (scope.lexical.contains(&name) && !catch_param)
                        || (scope.flags & FUNCTION == 0 && scope.functions.contains(&name))
                    {
                        redeclared = true;
                        break;
                    }
                    scope.var.push(name);
                    if scope.flags & VAR_SCOPE != 0 {
                        break;
                    }
                }
                redeclared
            }
        };
        if redeclared {
            self.error(at, format!("Identifier '{name}' has already been declared"));
        }
    }

    /// The binding a function declaration makes in the current scope:
    /// function-scoped at the top of a function body, block-scoped
    /// everywhere else (a module is strict code).
    fn function_bind(&self) -> Bind {
        if self.scopes.last().is_some_and(|s| s.flags & FUNCTION != 0) {
            Bind::Var
        } else {
            Bind::Lexical
        }
    }

    fn declare_pattern(&mut self, pattern: &BindingPattern<'t>, bind: Bind) {
        let mut ids = Vec::new();
        pattern_ids(pattern, &mut ids);
        for id in ids {
            self.declare(id.name.as_str(), bind, id.span.start);
        }
    }

    /// The body of an `if`, a loop or a label is a statement. In strict
    /// code a function or class declaration is none, and acorn stops at
    /// its first token.
    fn declaration_as_body(&mut self, body: &Statement<'_>) {
        if matches!(
            body,
            Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_)
        ) {
            self.error(body.span().start, "Unexpected token");
        }
    }

    /// acorn's `checkUnreserved`, for every identifier it reads.
    fn check_unreserved(&mut self, name: &str, at: u32) {
        let ctx = self.context();
        let message = if ctx.generator && name == "yield" {
            "Cannot use 'yield' as identifier inside a generator".to_string()
        } else if ctx.field_init && name == "arguments" {
            "Cannot use 'arguments' in class field initializer".to_string()
        } else if ctx.static_block && (name == "arguments" || name == "await") {
            format!("Cannot use {name} in class static initialization block")
        } else if name == "enum" || STRICT_RESERVED.contains(&name) {
            format!("The keyword '{name}' is reserved")
        } else {
            return;
        };
        self.error(at, message);
    }

    /// The words between `from` and `to` that read as identifiers, with
    /// their offsets: the modifiers in front of a class member or a
    /// parameter.
    fn words(&self, from: u32, to: u32) -> Vec<(&'s str, u32)> {
        let mut words = Vec::new();
        let mut at = from;
        while at < to {
            at = self.next_token(at);
            if at >= to {
                break;
            }
            let rest = &self.text[at as usize..to as usize];
            let len = rest
                .char_indices()
                .find(|&(i, c)| {
                    !(if i == 0 {
                        oxc_syntax::identifier::is_identifier_start(c)
                    } else {
                        oxc_syntax::identifier::is_identifier_part(c)
                    })
                })
                .map_or(rest.len(), |(i, _)| i);
            if len == 0 {
                break;
            }
            words.push((&rest[..len], at));
            at += len as u32;
        }
        words
    }

    /// Modifiers in front of a class member.
    fn check_member_modifiers(&mut self, element: Span, decorators: &[Decorator<'_>], key: u32) {
        let start = decorators.last().map_or(element.start, |d| d.span.end);
        let words = self.words(start, key);
        let modifiers: Vec<(&str, u32)> = words
            .iter()
            .copied()
            .take_while(|(w, _)| MEMBER_MODIFIERS.contains(w))
            .collect();
        if self.js() {
            // Plain acorn reads the first TypeScript-only modifier as the
            // member's name and stops at the token after it.
            if let Some(i) = modifiers
                .iter()
                .position(|(w, _)| TS_ONLY_MEMBER_MODIFIERS.contains(w))
            {
                let after = words.get(i + 1).map_or(key, |&(_, at)| at);
                self.error(after, "Unexpected token");
            }
        }
    }

    /// A function's parameters and body, in the scope they share.
    fn function_parts(
        &mut self,
        params: &FormalParameters<'t>,
        body: Option<&FunctionBody<'t>>,
        start: u32,
    ) {
        self.visit_formal_parameters(params);
        // Parameter names: function-scoped, never repeated.
        let mut ids = Vec::new();
        for param in &params.items {
            pattern_ids(&param.pattern, &mut ids);
        }
        if let Some(rest) = &params.rest {
            pattern_ids(&rest.rest.argument, &mut ids);
        }
        let mut seen: Vec<&str> = Vec::new();
        for id in ids {
            let name = id.name.as_str();
            if seen.contains(&name) {
                self.error(id.span.start, "Argument name clash");
            }
            seen.push(name);
            self.declare(name, Bind::Var, id.span.start);
        }
        if let Some(body) = body {
            let simple = params.rest.is_none()
                && params.items.iter().all(|p| {
                    matches!(p.pattern, BindingPattern::BindingIdentifier(_))
                        && p.initializer.is_none()
                });
            if !simple
                && body.directives.iter().any(|d| {
                    self.span_text(d.expression.span) == "'use strict'"
                        || self.span_text(d.expression.span) == "\"use strict\""
                })
            {
                self.error(
                    start,
                    "Illegal 'use strict' directive in function with non-simple parameter list",
                );
            }
            for directive in &body.directives {
                self.visit_string_literal(&directive.expression);
            }
            self.visit_statements(&body.statements);
        }
    }

    fn span_text(&self, span: Span) -> &'s str {
        self.text
            .get(span.start as usize..span.end as usize)
            .unwrap_or_default()
    }

    /// Enter a function-like context (function, arrow, static block,
    /// field initializer).
    fn with_context<F: FnOnce(&mut Self)>(&mut self, ctx: Context<'t>, flags: u8, f: F) {
        self.scopes.push(Scope {
            flags,
            context: Some(ctx),
            ..Scope::default()
        });
        f(self);
        self.pop_scope();
    }

    /// Octal escapes (and `\8`, `\9`) in a string literal.
    fn check_string_escapes(&mut self, span: Span) {
        let raw = self.span_text(span);
        let bytes = raw.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'\\' {
                i += 1;
                continue;
            }
            let Some(&next) = bytes.get(i + 1) else {
                return;
            };
            match next {
                b'8' | b'9' => {
                    self.error(span.start + i as u32 + 1, "Invalid escape sequence");
                    return;
                }
                b'1'..=b'7' => {
                    self.error(span.start + i as u32, "Octal literal in strict mode");
                    return;
                }
                b'0' if bytes.get(i + 2).is_some_and(u8::is_ascii_digit) => {
                    self.error(span.start + i as u32, "Octal literal in strict mode");
                    return;
                }
                _ => {}
            }
            i += 2;
        }
    }

    fn check_break_continue(
        &mut self,
        label: Option<&LabelIdentifier<'_>>,
        is_break: bool,
        at: u32,
    ) {
        let name = label.map(|l| l.name.as_str());
        let found = self.context().labels.iter().any(|lab| {
            (name.is_none() || lab.name == name)
                && ((lab.kind != LabelKind::Other && (is_break || lab.kind == LabelKind::Loop))
                    || (name.is_some() && is_break))
        });
        if !found {
            let keyword = if is_break { "break" } else { "continue" };
            self.error(at, format!("Unsyntactic {keyword}"));
        }
    }

    fn with_loop_label<F: FnOnce(&mut Self)>(&mut self, kind: LabelKind, f: F) {
        if let Some(ctx) = self.context_mut() {
            ctx.labels.push(Label { name: None, kind });
        }
        f(self);
        if let Some(ctx) = self.context_mut() {
            ctx.labels.pop();
        }
    }

    /// A class element's private name declaration.
    fn declare_private(&mut self, name: &'t str, state: &'static str, at: u32) {
        let Some(class) = self.classes.last_mut() else {
            return;
        };
        match class.declared.iter_mut().find(|(n, _)| *n == name) {
            None => class.declared.push((name, state)),
            Some((_, current)) => {
                let pairs = matches!(
                    (*current, state),
                    ("iget", "iset") | ("iset", "iget") | ("sget", "sset") | ("sset", "sget")
                );
                if pairs {
                    *current = "true";
                } else {
                    self.error(
                        at,
                        format!("Identifier '#{name}' has already been declared"),
                    );
                }
            }
        }
    }

    fn use_private(&mut self, name: &'t str, at: u32) {
        match self.classes.last_mut() {
            Some(class) => class.used.push((name, at)),
            None => self.error(
                at,
                format!("Private field '#{name}' must be declared in an enclosing class"),
            ),
        }
    }
}

/// acorn's message for the regular expression literal `raw` (from its
/// opening `/` through its flags) when the pattern is invalid.
pub(crate) fn regexp_pattern_error(raw: &str) -> Option<String> {
    let close = raw.rfind('/').filter(|&i| i > 0)?;
    let (pattern, flags) = (&raw[1..close], &raw[close + 1..]);
    let reason = acorn_rs::regexp_pattern_error(pattern, flags)?;
    if reason == acorn_rs::STACK_EXHAUSTED {
        return Some(reason.to_string());
    }
    Some(format!("Invalid regular expression: /{pattern}/: {reason}"))
}

/// The binding identifiers of a pattern, in source order.
/// Whether acorn's `parseExprOps` goes on to read a binary operator
/// after `expr` as a class field's initializer: everything but an
/// arrow function with a block body, which it returns as is. An arrow's
/// expression body is read the same way.
fn continues_with_operator(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::ArrowFunctionExpression(arrow) => {
            arrow.get_expression().is_some_and(continues_with_operator)
        }
        Expression::ConditionalExpression(c) => continues_with_operator(&c.alternate),
        Expression::AssignmentExpression(a) => continues_with_operator(&a.right),
        _ => true,
    }
}

fn pattern_ids<'b, 'a>(pattern: &'b BindingPattern<'a>, out: &mut Vec<&'b BindingIdentifier<'a>>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => out.push(id),
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                pattern_ids(&prop.value, out);
            }
            if let Some(rest) = &obj.rest {
                pattern_ids(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for el in arr.elements.iter().flatten() {
                pattern_ids(el, out);
            }
            if let Some(rest) = &arr.rest {
                pattern_ids(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(assign) => pattern_ids(&assign.left, out),
    }
}

pub(crate) use svn_parser::acorn::next_token_start;

impl<'a> Visit<'a> for Checker<'_, 'a> {
    fn visit_statement(&mut self, it: &Statement<'a>) {
        // oxc's TypeScript reading accepts a module declaration inside a
        // block or function and leaves the error to a later pass.
        let module_declaration = matches!(
            it,
            Statement::ImportDeclaration(_)
                | Statement::ExportNamedDeclaration(_)
                | Statement::ExportDefaultDeclaration(_)
                | Statement::ExportAllDeclaration(_)
        );
        let start = it.span().start;
        // It reads `export` before a nested declaration as a modifier
        // and keeps only the declaration.
        let exported = self.text.get(start as usize..).is_some_and(|rest| {
            rest.strip_prefix("export")
                .is_some_and(|after| !after.starts_with(oxc_syntax::identifier::is_identifier_part))
        });
        if (module_declaration || exported) && !self.top_level.contains(&start) {
            self.error(
                start,
                "'import' and 'export' may only appear at the top level",
            );
        }
        if !self.settled(it.span().start) {
            walk::walk_statement(self, it);
        }
    }

    fn visit_block_statement(&mut self, it: &BlockStatement<'a>) {
        self.push_scope(0);
        self.visit_statements(&it.body);
        self.pop_scope();
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        if it.declare {
            return;
        }
        if matches!(
            it.kind,
            VariableDeclarationKind::Using | VariableDeclarationKind::AwaitUsing
        ) && let Some(first) = it.declarations.first()
        {
            // Neither parser knows `using`: it is read as an identifier
            // and the binding after it is unexpected.
            self.error(first.id.span().start, "Unexpected token");
        }
        let saved = self.declaration_kind.replace(it.kind);
        for declarator in &it.declarations {
            self.visit_variable_declarator(declarator);
        }
        self.declaration_kind = saved;
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        self.visit_binding_pattern(&it.id);
        let bind = match self.declaration_kind {
            Some(VariableDeclarationKind::Var) => Bind::Var,
            _ => Bind::Lexical,
        };
        self.declare_pattern(&it.id, bind);
        if let Some(init) = &it.init {
            self.visit_expression(init);
        }
    }

    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let is_declaration = matches!(
            it.r#type,
            FunctionType::FunctionDeclaration | FunctionType::TSDeclareFunction
        );
        if it.declare {
            return;
        }
        if let Some(id) = &it.id {
            self.visit_binding_identifier(id);
        }
        // Plain acorn declares the name before reading the function;
        // acorn-typescript after its body, and only for a function with
        // a body (overload signatures declare nothing).
        let declared_bind = self.function_bind();
        if is_declaration
            && !self.mode.ts
            && let Some(id) = &it.id
        {
            self.declare(id.name.as_str(), declared_bind, id.span.start);
        }
        // Methods are read through `method`; any other function
        // resets what the enclosing code allows.
        let _ = flags;
        let ctx = Context {
            function: true,
            is_async: it.r#async,
            generator: it.generator,
            ..Context::default()
        };
        let outer = (
            self.in_parameter_default,
            self.parameter_await_is_expression,
        );
        self.in_parameter_default = false;
        self.parameter_await_is_expression = it.r#async;
        self.with_context(ctx, FUNCTION, |this| {
            this.function_parts(&it.params, it.body.as_deref(), it.span.start);
        });
        (
            self.in_parameter_default,
            self.parameter_await_is_expression,
        ) = outer;
        if is_declaration
            && self.mode.ts
            && it.body.is_some()
            && let Some(id) = &it.id
        {
            let before = self.best.clone();
            self.declare(id.name.as_str(), declared_bind, id.span.start);
            // Raised once the body has been read.
            if self.best != before
                && let Some(best) = self.best.as_mut()
            {
                best.detect = it.span.end;
                if before.as_ref().is_some_and(|b| b.detect <= best.detect) {
                    self.best = before;
                }
            }
        }
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let parent = self.context();
        let ctx = Context {
            function: true,
            is_async: it.r#async,
            generator: false,
            allow_super: parent.allow_super,
            allow_direct_super: parent.allow_direct_super,
            field_init: parent.field_init,
            static_block: false,
            labels: Vec::new(),
        };
        // acorn reads an arrow function's parameters as an expression
        // of the code around it before it sees the arrow.
        let await_is_expression =
            it.r#async || parent.is_async || (!parent.function && !parent.field_init);
        let outer = (
            self.in_parameter_default,
            self.parameter_await_is_expression,
        );
        self.in_parameter_default = false;
        self.parameter_await_is_expression = await_is_expression;
        self.with_context(ctx, FUNCTION, |this| match &it.body {
            ArrowFunctionBody::FunctionBody(body) => {
                this.function_parts(&it.params, Some(body), it.span.start);
            }
            body => {
                this.function_parts(&it.params, None, it.span.start);
                if let Some(expression) = body.as_expression() {
                    this.visit_expression(expression);
                }
            }
        });
        (
            self.in_parameter_default,
            self.parameter_await_is_expression,
        ) = outer;
    }

    fn visit_conditional_expression(&mut self, it: &ConditionalExpression<'a>) {
        walk::walk_conditional_expression(self, it);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        walk::walk_import_expression(self, it);
    }

    fn visit_directive(&mut self, it: &Directive<'a>) {
        self.check_string_escapes(it.expression.span);
    }

    fn visit_yield_expression(&mut self, it: &YieldExpression<'a>) {
        if self.in_parameter_default {
            self.error(it.span.start, "Yield expression cannot be a default value");
        }
        walk::walk_yield_expression(self, it);
    }

    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        self.declaration_as_body(&it.consequent);
        if let Some(alternate) = &it.alternate {
            self.declaration_as_body(alternate);
        }
        walk::walk_if_statement(self, it);
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        for decorator in &it.decorators {
            self.visit_decorator(decorator);
        }
        self.visit_binding_pattern(&it.pattern);
        if let Some(init) = &it.initializer {
            let outer = std::mem::replace(&mut self.in_parameter_default, true);
            self.visit_expression(init);
            self.in_parameter_default = outer;
        }
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        if it.declare {
            return;
        }
        for decorator in &it.decorators {
            self.visit_decorator(decorator);
        }
        if let Some(id) = &it.id {
            self.visit_binding_identifier(id);
            if it.r#type == oxc_ast::ast::ClassType::ClassDeclaration {
                self.declare(id.name.as_str(), Bind::Lexical, id.span.start);
            }
        }
        if let Some(heritage) = &it.heritage {
            self.visit_expression(&heritage.expression);
        }
        if self.js() && !it.implements.is_empty() {
            let at = self.next_token(it.id.as_ref().map_or(it.span.start, |id| id.span.end));
            self.error(at, "Unexpected token");
        }
        let derived = it.heritage.is_some();
        self.classes.push(PrivateNames::default());
        let mut had_constructor = false;
        let mut previous = None;
        for element in &it.body.body {
            if self.settled(element.span().start) {
                break;
            }
            if let Some(previous) = previous {
                self.escaped_operator_after_field(previous, element);
            }
            self.class_element(element, derived, &mut had_constructor);
            previous = Some(element);
        }
        let Some(names) = self.classes.pop() else {
            return;
        };
        for (name, at) in names.used {
            if names.declared.iter().any(|(n, _)| *n == name) {
                continue;
            }
            match self.classes.last_mut() {
                Some(parent) => parent.used.push((name, at)),
                None => self.record(Candidate {
                    detect: it.body.span.end,
                    at: Position::At(at),
                    message: format!(
                        "Private field '#{name}' must be declared in an enclosing class"
                    ),
                    token_start: None,
                }),
            }
        }
    }

    fn visit_static_block(&mut self, it: &StaticBlock<'a>) {
        let ctx = Context {
            allow_super: true,
            static_block: true,
            ..Context::default()
        };
        self.with_context(ctx, STATIC_BLOCK, |this| {
            this.visit_statements(&it.body);
        });
    }

    fn visit_catch_clause(&mut self, it: &CatchClause<'a>) {
        let Some(param) = &it.param else {
            self.visit_block_statement(&it.body);
            return;
        };
        let simple = matches!(param.pattern, BindingPattern::BindingIdentifier(_));
        self.push_scope(if simple { SIMPLE_CATCH } else { 0 });
        self.visit_binding_pattern(&param.pattern);
        self.declare_pattern(
            &param.pattern,
            if simple {
                Bind::SimpleCatch
            } else {
                Bind::Lexical
            },
        );
        // The body's statements share the parameter's scope.
        self.visit_statements(&it.body.body);
        self.pop_scope();
    }

    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.push_scope(0);
        if let Some(init) = &it.init {
            match init {
                ForStatementInit::VariableDeclaration(d) => self.visit_variable_declaration(d),
                _ => {
                    if let Some(e) = init.as_expression() {
                        self.visit_expression(e);
                    }
                }
            }
        }
        if let Some(test) = &it.test {
            self.visit_expression(test);
        }
        if let Some(update) = &it.update {
            self.visit_expression(update);
        }
        self.declaration_as_body(&it.body);
        self.with_loop_label(LabelKind::Loop, |this| this.visit_statement(&it.body));
        self.pop_scope();
    }

    fn visit_for_in_statement(&mut self, it: &ForInStatement<'a>) {
        self.for_each(&it.left, &it.right, &it.body, "for-in");
    }

    fn visit_for_of_statement(&mut self, it: &ForOfStatement<'a>) {
        self.for_each(&it.left, &it.right, &it.body, "for-of");
    }

    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.visit_expression(&it.test);
        self.declaration_as_body(&it.body);
        self.with_loop_label(LabelKind::Loop, |this| this.visit_statement(&it.body));
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
        self.declaration_as_body(&it.body);
        self.with_loop_label(LabelKind::Loop, |this| this.visit_statement(&it.body));
        self.visit_expression(&it.test);
    }

    fn visit_switch_statement(&mut self, it: &SwitchStatement<'a>) {
        self.visit_expression(&it.discriminant);
        self.push_scope(0);
        self.with_loop_label(LabelKind::Switch, |this| {
            for case in &it.cases {
                if let Some(test) = &case.test {
                    this.visit_expression(test);
                }
                this.visit_statements(&case.consequent);
            }
        });
        self.pop_scope();
    }

    fn visit_labeled_statement(&mut self, it: &LabeledStatement<'a>) {
        let name = it.label.name.as_str();
        if self.context().labels.iter().any(|l| l.name == Some(name)) {
            self.error(
                it.label.span.start,
                format!("Label '{name}' is already declared"),
            );
        }
        self.check_unreserved(name, it.label.span.start);
        let mut body = &it.body;
        while let Statement::LabeledStatement(inner) = body {
            body = &inner.body;
        }
        self.declaration_as_body(&it.body);
        let kind = match body {
            Statement::ForStatement(_)
            | Statement::ForInStatement(_)
            | Statement::ForOfStatement(_)
            | Statement::WhileStatement(_)
            | Statement::DoWhileStatement(_) => LabelKind::Loop,
            Statement::SwitchStatement(_) => LabelKind::Switch,
            _ => LabelKind::Other,
        };
        if let Some(ctx) = self.context_mut() {
            ctx.labels.push(Label {
                name: Some(name),
                kind,
            });
        }
        self.visit_statement(&it.body);
        if let Some(ctx) = self.context_mut() {
            ctx.labels.pop();
        }
    }

    fn visit_break_statement(&mut self, it: &BreakStatement<'a>) {
        self.check_break_continue(it.label.as_ref(), true, it.span.start);
    }

    fn visit_continue_statement(&mut self, it: &ContinueStatement<'a>) {
        self.check_break_continue(it.label.as_ref(), false, it.span.start);
    }

    fn visit_with_statement(&mut self, it: &WithStatement<'a>) {
        self.error(it.span.start, "'with' in strict mode");
    }

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        if it.import_kind == ImportOrExportKind::Type {
            return;
        }
        if it.phase.is_some() {
            // `defer` / `source` is read as the default binding and what
            // follows it is unexpected.
            let at = it
                .specifiers
                .as_ref()
                .and_then(|s| s.first())
                .map_or(it.source.span.start, |s| s.span().start);
            self.error(at, "Unexpected token");
        }
        for specifier in it.specifiers.iter().flatten() {
            let local = match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    if s.import_kind == ImportOrExportKind::Type {
                        continue;
                    }
                    &s.local
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local,
            };
            self.visit_binding_identifier(local);
            self.declare(local.name.as_str(), Bind::Lexical, local.span.start);
        }
    }

    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        self.check_unreserved(it.name.as_str(), it.span.start);
    }

    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        let name = it.name.as_str();
        self.check_unreserved(name, it.span.start);
        if name == "eval" || name == "arguments" {
            self.error(it.span.start, format!("Binding {name} in strict mode"));
        }
    }

    fn visit_label_identifier(&mut self, it: &LabelIdentifier<'a>) {
        self.check_unreserved(it.name.as_str(), it.span.start);
    }

    fn visit_simple_assignment_target(&mut self, it: &SimpleAssignmentTarget<'a>) {
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = it {
            let name = id.name.as_str();
            self.check_unreserved(name, id.span.start);
            if name == "eval" || name == "arguments" {
                self.error(id.span.start, format!("Assigning to {name} in strict mode"));
            }
            return;
        }
        walk::walk_simple_assignment_target(self, it);
    }

    fn visit_assignment_target_property_identifier(
        &mut self,
        it: &AssignmentTargetPropertyIdentifier<'a>,
    ) {
        let name = it.binding.name.as_str();
        self.check_unreserved(name, it.binding.span.start);
        if name == "eval" || name == "arguments" {
            self.error(
                it.binding.span.start,
                format!("Assigning to {name} in strict mode"),
            );
        }
        if let Some(init) = &it.init {
            self.visit_expression(init);
        }
    }

    fn visit_unary_expression(&mut self, it: &UnaryExpression<'a>) {
        if it.operator == UnaryOperator::Delete
            && matches!(
                it.argument.get_inner_expression(),
                Expression::Identifier(_)
            )
        {
            self.error(it.span.start, "Deleting local variable in strict mode");
        }
        if it.operator == UnaryOperator::Delete {
            // `get_inner_expression` already strips parentheses.
            let target = it.argument.get_inner_expression();
            if let Expression::ChainExpression(chain) = target
                && let ChainElement::PrivateFieldExpression(_) = &chain.expression
            {
                self.error(it.span.start, "Private fields can not be deleted");
            }
            if matches!(target, Expression::PrivateFieldExpression(_)) {
                self.error(it.span.start, "Private fields can not be deleted");
            }
        }
        walk::walk_unary_expression(self, it);
    }

    // TypeScript's transpile prints legacy octal literals anew.
    fn visit_numeric_literal(&mut self, _it: &NumericLiteral<'a>) {}

    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.check_string_escapes(it.span);
    }

    fn visit_reg_exp_literal(&mut self, it: &RegExpLiteral<'a>) {
        if let Some(message) = regexp_pattern_error(self.span_text(it.span)) {
            self.record(Candidate::at(it.span.start + 1, message).in_token(it.span.start));
        }
    }

    fn visit_super(&mut self, it: &Super) {
        if !self.context().allow_super {
            self.error(it.span.start, "'super' keyword outside a method");
        }
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Super(s) = &it.callee {
            if !self.context().allow_super {
                self.error(s.span.start, "'super' keyword outside a method");
            } else if !self.context().allow_direct_super {
                self.error(
                    s.span.start,
                    "super() call outside constructor of a subclass",
                );
            }
            for arg in &it.arguments {
                self.visit_argument(arg);
            }
            return;
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_await_expression(&mut self, it: &AwaitExpression<'a>) {
        let ctx = self.context();
        if self.in_parameter_default && self.parameter_await_is_expression {
            self.error(it.span.start, "Await expression cannot be a default value");
        } else if ctx.static_block {
            self.error(
                it.span.start,
                "Cannot use await in class static initialization block",
            );
        } else if (ctx.function && !ctx.is_async) || (!ctx.function && ctx.field_init) {
            self.error(
                it.span.start,
                "Cannot use keyword 'await' outside an async function",
            );
        }
        walk::walk_await_expression(self, it);
    }

    fn visit_private_field_expression(&mut self, it: &PrivateFieldExpression<'a>) {
        self.visit_expression(&it.object);
        self.use_private(it.field.name.as_str(), it.field.span.start);
    }

    fn visit_tagged_template_expression(&mut self, it: &TaggedTemplateExpression<'a>) {
        // acorn rejects an optional chain as the tag at the template.
        // TypeScript's parser accepts it (the checker reports it), and
        // TypeScript's transpile prints it unchanged, so the compiler
        // still sees it in a transpiled script.
        // With type arguments acorn-typescript reads the template as
        // part of the chain.
        let typed = self.mode.ts && it.type_arguments.is_some();
        if !typed && crate::transpile_sensitive::tag_has_optional_chain(&it.tag) {
            self.error(
                it.quasi.span.start,
                "Optional chaining cannot appear in the tag of tagged template expressions",
            );
        }
        walk::walk_tagged_template_expression(self, it);
    }

    fn visit_private_in_expression(&mut self, it: &PrivateInExpression<'a>) {
        // Outside every class acorn does not read `#x in` at all: the
        // private name is an unexpected token.
        if self.classes.is_empty() {
            self.error(it.left.span.start, "Unexpected token");
        } else {
            self.use_private(it.left.name.as_str(), it.left.span.start);
        }
        self.visit_expression(&it.right);
    }

    fn visit_object_expression(&mut self, it: &ObjectExpression<'a>) {
        let mut proto = false;
        for prop in &it.properties {
            let ObjectPropertyKind::ObjectProperty(p) = prop else {
                self.visit_object_property_kind(prop);
                continue;
            };
            if !p.computed && !p.shorthand && !p.method && p.kind == PropertyKind::Init {
                let is_proto = match &p.key {
                    PropertyKey::StaticIdentifier(id) => id.name == "__proto__",
                    PropertyKey::StringLiteral(s) => s.value == "__proto__",
                    _ => false,
                };
                if is_proto {
                    if proto {
                        self.error(p.key.span().start, "Redefinition of __proto__ property");
                    }
                    proto = true;
                }
            }
            self.visit_property_key(&p.key);
            if p.method || p.kind != PropertyKind::Init {
                // Object methods may use `super`.
                if let Expression::FunctionExpression(f) = &p.value {
                    self.method(f, false);
                    continue;
                }
            }
            self.visit_expression(&p.value);
        }
    }

    fn visit_decorator(&mut self, it: &Decorator<'a>) {
        if self.js() {
            self.error(it.span.start, "Unexpected character '@'");
        }
    }

    fn visit_ts_as_expression(&mut self, it: &TSAsExpression<'a>) {
        self.visit_expression(&it.expression);
        if self.js() {
            let at = self.next_token(it.expression.span().end);
            self.error(at, "Unexpected token");
        }
    }

    fn visit_ts_satisfies_expression(&mut self, it: &TSSatisfiesExpression<'a>) {
        self.visit_expression(&it.expression);
        if self.js() {
            let at = self.next_token(it.expression.span().end);
            self.error(at, "Unexpected token");
        }
    }

    // TypeScript's transpile drops type declarations.
    fn visit_ts_interface_declaration(&mut self, _it: &TSInterfaceDeclaration<'a>) {}
    fn visit_ts_type_alias_declaration(&mut self, _it: &TSTypeAliasDeclaration<'a>) {}

    fn visit_ts_enum_declaration(&mut self, it: &TSEnumDeclaration<'a>) {
        // acorn-typescript declares nothing for an enum.
        if !it.declare {
            self.transpiled_var(it.id.name.as_str(), it.id.span.start);
        }
        for member in &it.body.members {
            if let Some(initializer) = &member.initializer {
                self.visit_expression(initializer);
            }
        }
    }

    fn visit_ts_namespace_declaration(&mut self, it: &TSNamespaceDeclaration<'a>) {
        if !crate::rules::transpile_positions::namespace_removed(it) {
            self.transpiled_var(it.id.name.as_str(), it.id.span.start);
            self.namespace_body(&it.body);
        }
    }

    // TypeScript's transpile drops `declare module` and `declare global`.
    fn visit_ts_external_module_declaration(&mut self, _it: &TSExternalModuleDeclaration<'a>) {}

    fn visit_ts_global_declaration(&mut self, _it: &TSGlobalDeclaration<'a>) {}

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        if it.import_kind == ImportOrExportKind::Type {
            return;
        }
        self.declare(it.id.name.as_str(), Bind::Lexical, it.id.span.start);
    }

    // Type syntax holds nothing the checks look at.
    fn visit_ts_type(&mut self, _it: &TSType<'a>) {}
    fn visit_ts_type_annotation(&mut self, _it: &TSTypeAnnotation<'a>) {}
    fn visit_ts_type_parameter_declaration(&mut self, _it: &TSTypeParameterDeclaration<'a>) {}
    fn visit_ts_type_parameter_instantiation(&mut self, _it: &TSTypeParameterInstantiation<'a>) {}
    fn visit_ts_class_implements(&mut self, _it: &TSClassImplements<'a>) {}
}

impl<'a> Checker<'_, 'a> {
    fn for_each(
        &mut self,
        left: &ForStatementLeft<'a>,
        right: &Expression<'a>,
        body: &Statement<'a>,
        kind: &str,
    ) {
        self.push_scope(0);
        match left {
            ForStatementLeft::VariableDeclaration(d) => {
                if d.declarations.iter().any(|decl| decl.init.is_some()) {
                    self.error(
                        d.span.start,
                        format!("{kind} loop variable declaration may not have an initializer"),
                    );
                }
                self.visit_variable_declaration(d);
            }
            _ => {
                if let Some(target) = left.as_assignment_target() {
                    self.visit_assignment_target(target);
                }
            }
        }
        self.visit_expression(right);
        self.declaration_as_body(body);
        self.with_loop_label(LabelKind::Loop, |this| this.visit_statement(body));
        self.pop_scope();
    }

    /// The `var` TypeScript's transpile writes for an enum or namespace,
    /// unless an earlier declaration of the name makes it merge into
    /// that one.
    fn transpiled_var(&mut self, name: &'a str, at: u32) {
        let declared = self.scopes.last().is_some_and(|s| {
            s.var.contains(&name) || s.lexical.contains(&name) || s.functions.contains(&name)
        });
        if !declared {
            self.declare(name, Bind::Var, at);
        }
    }

    fn namespace_body(&mut self, body: &TSNamespaceDeclarationBody<'a>) {
        self.push_scope(0);
        match body {
            TSNamespaceDeclarationBody::TSModuleBlock(block) => self.visit_statements(&block.body),
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(inner) => {
                self.namespace_body(&inner.body);
            }
        }
        self.pop_scope();
    }

    /// A method's function: `super` allowed, `super()` only in a
    /// derived class's constructor. acorn-typescript reads parameter
    /// modifiers in any method.
    fn method(&mut self, f: &Function<'a>, direct_super: bool) {
        let ctx = Context {
            function: true,
            is_async: f.r#async,
            generator: f.generator,
            allow_super: true,
            allow_direct_super: direct_super,
            ..Context::default()
        };
        self.with_context(ctx, FUNCTION, |this| {
            this.function_parts(&f.params, f.body.as_deref(), f.span.start);
        });
    }

    fn class_element(
        &mut self,
        element: &ClassElement<'a>,
        derived: bool,
        had_constructor: &mut bool,
    ) {
        match element {
            ClassElement::MethodDefinition(m) => {
                for decorator in &m.decorators {
                    self.visit_decorator(decorator);
                }
                self.check_member_modifiers(m.span, &m.decorators, m.key.span().start);
                if m.kind == MethodDefinitionKind::Constructor && m.value.body.is_some() {
                    if *had_constructor {
                        self.error(
                            m.key.span().start,
                            "Duplicate constructor in the same class",
                        );
                    }
                    *had_constructor = true;
                }
                if let PropertyKey::PrivateIdentifier(p) = &m.key {
                    let state = match (m.kind, m.r#static) {
                        (MethodDefinitionKind::Get, false) => "iget",
                        (MethodDefinitionKind::Set, false) => "iset",
                        (MethodDefinitionKind::Get, true) => "sget",
                        (MethodDefinitionKind::Set, true) => "sset",
                        _ => "true",
                    };
                    self.declare_private(p.name.as_str(), state, p.span.start);
                } else {
                    self.visit_property_key(&m.key);
                }
                let ctor = m.kind == MethodDefinitionKind::Constructor;
                self.method(&m.value, ctor && derived);
            }
            ClassElement::PropertyDefinition(p) => {
                for decorator in &p.decorators {
                    self.visit_decorator(decorator);
                }
                self.check_member_modifiers(p.span, &p.decorators, p.key.span().start);
                if let PropertyKey::PrivateIdentifier(id) = &p.key {
                    self.declare_private(id.name.as_str(), "true", id.span.start);
                } else {
                    self.visit_property_key(&p.key);
                }
                if let Some(value) = &p.value {
                    let ctx = Context {
                        allow_super: true,
                        field_init: true,
                        ..Context::default()
                    };
                    self.with_context(ctx, 0, |this| this.visit_expression(value));
                }
            }
            ClassElement::AccessorProperty(p) => {
                for decorator in &p.decorators {
                    self.visit_decorator(decorator);
                }
                if self.js() {
                    // Plain acorn reads `accessor` as the field's name.
                    let start = p.decorators.last().map_or(p.span.start, |d| d.span.end);
                    let words = self.words(start, p.key.span().start);
                    if let Some(i) = words.iter().position(|(w, _)| *w == "accessor") {
                        let after = words.get(i + 1).map_or(p.key.span().start, |&(_, at)| at);
                        self.error(after, "Unexpected token");
                    }
                } else {
                    self.check_member_modifiers(p.span, &p.decorators, p.key.span().start);
                }
                if let PropertyKey::PrivateIdentifier(id) = &p.key {
                    self.declare_private(id.name.as_str(), "true", id.span.start);
                } else {
                    self.visit_property_key(&p.key);
                }
                if let Some(value) = &p.value {
                    let ctx = Context {
                        allow_super: true,
                        field_init: true,
                        ..Context::default()
                    };
                    self.with_context(ctx, 0, |this| this.visit_expression(value));
                }
            }
            ClassElement::StaticBlock(b) => self.visit_static_block(b),
            ClassElement::TSIndexSignature(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    fn first(text: &str, ts: bool) -> Option<(u32, String)> {
        let allocator = Allocator::default();
        let source_type = SourceType::default().with_module(true).with_typescript(ts);
        let parsed = Parser::new(&allocator, text, source_type).parse();
        let mode = Mode { ts };
        first_early_error(&parsed.program, text, mode).map(|c| {
            let at = match c.at {
                Position::At(at) | Position::ColumnOf(at) => at,
                Position::Nowhere => 0,
            };
            (at, c.message)
        })
    }

    fn js(text: &str) -> Option<(u32, String)> {
        first(text, false)
    }

    #[test]
    fn redeclarations_follow_acorn_scopes() {
        assert_eq!(
            js("let x = 1;\nlet x = 2;"),
            Some((15, "Identifier 'x' has already been declared".into()))
        );
        assert_eq!(js("var x; var x;"), None);
        assert_eq!(js("let x; { let x; }"), None);
        assert_eq!(
            js("function f(a) { let a; }"),
            Some((20, "Identifier 'a' has already been declared".into()))
        );
        assert_eq!(js("function f(a) { var a; }"), None);
        assert_eq!(js("try {} catch (e) { var e; }"), None);
        assert_eq!(
            js("try {} catch (e) { let e; }"),
            Some((23, "Identifier 'e' has already been declared".into()))
        );
        assert_eq!(
            js("function f() {}\nfunction f() {}"),
            Some((25, "Identifier 'f' has already been declared".into()))
        );
        assert_eq!(js("function g() { function f() {} var f; }"), None);
        assert_eq!(
            js("import a from 'a';\nlet a = 1;"),
            Some((23, "Identifier 'a' has already been declared".into()))
        );
        assert_eq!(
            js("switch (1) { case 1: let a; case 2: let a; }"),
            Some((40, "Identifier 'a' has already been declared".into()))
        );
    }

    #[test]
    fn enums_and_namespaces_become_vars_when_transpiled() {
        let ts = |t| first(t, true);
        assert_eq!(
            ts("enum E { A }\nlet E = 1;"),
            Some((17, "Identifier 'E' has already been declared".into()))
        );
        assert_eq!(
            ts("namespace N { export const a = 1; }\nlet N = 1;"),
            Some((40, "Identifier 'N' has already been declared".into()))
        );
        // A namespace of types only prints nothing.
        assert_eq!(ts("namespace T { export type X = 1; }\nlet T = 1;"), None);
    }

    #[test]
    fn typescript_declarations_are_read_as_transpiled() {
        let ts = |t| first(t, true);
        assert_eq!(
            ts("function f(a: string): void;\nfunction f(a: any) {}"),
            None
        );
        assert_eq!(ts("interface A {}\nclass A {}"), None);
        assert_eq!(ts("type A = 1;\nconst A = 1;"), None);
        // Type declarations and `declare`d names are gone after the
        // transpile.
        assert_eq!(ts("type A = 1;\ntype A = 2;"), None);
        assert_eq!(ts("declare let x: number;\nlet x = 1;"), None);
    }

    #[test]
    fn parameters_labels_and_strict_mode() {
        assert_eq!(
            js("function f(a, a) {}"),
            Some((14, "Argument name clash".into()))
        );
        assert_eq!(js("break foo;"), Some((0, "Unsyntactic break".into())));
        assert_eq!(
            js("a: { continue a; }"),
            Some((5, "Unsyntactic continue".into()))
        );
        assert_eq!(js("a: for (;;) { break a; }"), None);
        assert_eq!(
            js("a: a: ;"),
            Some((3, "Label 'a' is already declared".into()))
        );
        assert_eq!(js("with (a) {}"), Some((0, "'with' in strict mode".into())));
        assert_eq!(
            js("let x; delete x;"),
            Some((7, "Deleting local variable in strict mode".into()))
        );
        assert_eq!(
            js("let s = '\\017';"),
            Some((9, "Octal literal in strict mode".into()))
        );
        assert_eq!(
            js("let yield = 1;"),
            Some((4, "The keyword 'yield' is reserved".into()))
        );
        assert_eq!(
            js("eval = 1;"),
            Some((0, "Assigning to eval in strict mode".into()))
        );
        assert_eq!(
            js("super.x;"),
            Some((0, "'super' keyword outside a method".into()))
        );
        assert_eq!(js("class A { m() { super.x; } }"), None);
    }

    #[test]
    fn classes() {
        assert_eq!(
            js("class A { constructor(){} constructor(){} }"),
            Some((26, "Duplicate constructor in the same class".into()))
        );
        assert_eq!(
            js("class A { m() { this.#x; } }"),
            Some((
                21,
                "Private field '#x' must be declared in an enclosing class".into()
            ))
        );
        assert_eq!(js("class A { #x; m(o) { return #x in o; } }"), None);
        assert_eq!(js("class A { get #x() { return 1 } set #x(v) {} }"), None);
        assert_eq!(
            js("class A { #x; #x; }"),
            Some((14, "Identifier '#x' has already been declared".into()))
        );
        assert_eq!(
            js("class A { x = arguments; }"),
            Some((
                14,
                "Cannot use 'arguments' in class field initializer".into()
            ))
        );
    }

    #[test]
    fn typescript_in_a_javascript_component() {
        assert_eq!(
            js("let a = b as any;"),
            Some((10, "Unexpected token".into()))
        );
        assert_eq!(
            js("class A { private x = 1; }"),
            Some((18, "Unexpected token".into()))
        );
        assert_eq!(
            js("class K { accessor x = 1 }"),
            Some((19, "Unexpected token".into()))
        );
        assert_eq!(
            js("@dec class A {}"),
            Some((0, "Unexpected character '@'".into()))
        );
    }
}
