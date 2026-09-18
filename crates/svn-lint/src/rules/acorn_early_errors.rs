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
//!   phases (`import defer`);
//! - acorn-typescript reads parameter modifiers only in class methods
//!   (elsewhere `private` is a reserved word and `readonly` an
//!   identifier), checks the order and compatibility of class member
//!   modifiers itself, and declares type aliases, interfaces and
//!   namespaces in scopes of its own.

use oxc_ast::ast::{
    ArrowFunctionBody, ArrowFunctionExpression, AssignmentTargetPropertyIdentifier,
    AwaitExpression, BindingIdentifier, BindingPattern, BlockStatement, BreakStatement,
    CallExpression, CatchClause, Class, ClassElement, ContinueStatement, Decorator,
    DoWhileStatement, Expression, ForInStatement, ForOfStatement, ForStatement, ForStatementInit,
    ForStatementLeft, FormalParameter, FormalParameters, Function, FunctionBody, FunctionType,
    IdentifierReference, ImportDeclaration, ImportDeclarationSpecifier, ImportOrExportKind,
    LabelIdentifier, LabeledStatement, MethodDefinitionKind, NumericLiteral, ObjectExpression,
    ObjectPropertyKind, PrivateFieldExpression, PrivateInExpression, Program, PropertyKey,
    PropertyKind, RegExpLiteral, SimpleAssignmentTarget, Statement, StaticBlock, StringLiteral,
    Super, SwitchStatement, TSAsExpression, TSClassImplements, TSEnumDeclaration,
    TSExternalModuleDeclaration, TSGlobalDeclaration, TSImportEqualsDeclaration,
    TSInterfaceDeclaration, TSNamespaceDeclaration, TSNamespaceDeclarationBody,
    TSSatisfiesExpression, TSType, TSTypeAliasDeclaration, TSTypeAnnotation,
    TSTypeParameterDeclaration, TSTypeParameterInstantiation, TaggedTemplateExpression,
    UnaryExpression, UnaryOperator, VariableDeclaration, VariableDeclarationKind,
    VariableDeclarator, WhileStatement, WithStatement,
};
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
    /// A preprocessor turned the script into JavaScript first: only
    /// what TypeScript's transpile keeps reaches the parser.
    pub transpiled: bool,
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
        best: None,
    };
    checker.scopes.push(Scope {
        flags: TOP,
        context: Some(Context::default()),
        ..Scope::default()
    });
    checker.visit_statements(&program.body);
    checker.best
}

/// Where the first bodiless function declaration (overload signature,
/// `declare function`) or method ends, when acorn-typescript reads the
/// script as written: from there on it believes it is inside a
/// function.
pub(crate) fn signature_scope_start(program: &Program<'_>, mode: Mode) -> Option<u32> {
    struct Finder {
        found: Option<u32>,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
            if it.body.is_none() {
                let end = it.span.end;
                if self.found.is_none_or(|f| end < f) {
                    self.found = Some(end);
                }
            }
            walk::walk_function(self, it, flags);
        }
    }
    if !mode.ts || mode.transpiled {
        return None;
    }
    let mut finder = Finder { found: None };
    finder.visit_program(program);
    finder.found
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
    types: Vec<&'t str>,
    /// Set on the scope of a function, static block or field
    /// initializer: what code directly inside it may do.
    context: Option<Context<'t>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bind {
    Var,
    Lexical,
    SimpleCatch,
    TsType,
    TsInterface,
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

const ACCESS_MODIFIERS: &[&str] = &["public", "private", "protected"];

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

/// Words acorn-typescript reads as parameter modifiers.
const PARAMETER_MODIFIERS: &[&str] = &["public", "private", "protected", "override", "readonly"];

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

    /// acorn-typescript reads the script as written (not transpiled).
    fn ts_as_written(&self) -> bool {
        self.mode.ts && !self.mode.transpiled
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

    /// acorn's `declareName`, with acorn-typescript's type bindings.
    fn declare(&mut self, name: &'t str, bind: Bind, at: u32) {
        let depth = self.scopes.len();
        let Some(scope) = self.scopes.last_mut() else {
            return;
        };
        let redeclared = match bind {
            Bind::TsType | Bind::TsInterface => {
                if bind == Bind::TsType && scope.types.contains(&name) {
                    self.error(at, format!("type '{name}' has already been declared."));
                    return;
                }
                scope.types.push(name);
                false
            }
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

    /// acorn-typescript's `tsParseModifiers` over the modifiers in
    /// front of a class member (`key` is where the member name starts).
    fn check_modifier_list(&mut self, words: &[(&str, u32)], allowed: &[&str], key: u32) {
        let mut modified: Vec<&str> = Vec::new();
        let mut accessibility = false;
        for (i, &(word, at)) in words.iter().enumerate() {
            if !allowed.contains(&word) {
                return;
            }
            // Where the parser stands once the modifier is read.
            let after = words.get(i + 1).map_or(key, |&(_, next)| next);
            let order = |before: &str, after: &str| {
                format!("'{before}' modifier must precede '{after}' modifier.")
            };
            let incompatible =
                |a: &str, b: &str| format!("'{a}' modifier cannot be used with '{b}' modifier.");
            let column_error = |message: String| Candidate {
                detect: at,
                at: Position::ColumnOf(at),
                message,
                token_start: None,
            };
            if ACCESS_MODIFIERS.contains(&word) {
                if accessibility {
                    self.error(after, "Accessibility modifier already seen.");
                    return;
                }
                for later in ["override", "static", "readonly", "accessor"] {
                    if modified.contains(&later) {
                        self.record(column_error(order(word, later)));
                        return;
                    }
                }
                accessibility = true;
            } else if word == "accessor" {
                if modified.contains(&word) {
                    self.error(after, format!("Duplicate modifier: '{word}'."));
                    return;
                }
                for other in ["readonly", "static", "override"] {
                    if modified.contains(&other) {
                        self.record(column_error(incompatible("accessor", other)));
                        return;
                    }
                }
                modified.push(word);
            } else {
                if modified.contains(&word) {
                    self.error(after, format!("Duplicate modifier: '{word}'."));
                    return;
                }
                for (before, later) in [
                    ("static", "readonly"),
                    ("static", "override"),
                    ("override", "readonly"),
                    ("abstract", "override"),
                ] {
                    if word == before && modified.contains(&later) {
                        self.record(column_error(order(before, later)));
                        return;
                    }
                }
                for (a, b) in [("declare", "override"), ("static", "abstract")] {
                    if (modified.contains(&a) && word == b) || (modified.contains(&b) && word == a)
                    {
                        self.record(column_error(incompatible(a, b)));
                        return;
                    }
                }
                modified.push(word);
            }
        }
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
        } else if self.ts_as_written() {
            self.check_modifier_list(&modifiers, MEMBER_MODIFIERS, key);
        }
    }

    /// A parameter written with modifiers. `modifiers_allowed`: the
    /// parser reads them here (class methods under acorn-typescript).
    fn check_parameter_modifiers(&mut self, param: &FormalParameter<'t>, modifiers_allowed: bool) {
        if !(param.accessibility.is_some() || param.readonly || param.r#override) {
            return;
        }
        if self.mode.transpiled {
            return;
        }
        let start = param
            .decorators
            .last()
            .map_or(param.span.start, |d| d.span.end);
        let pattern = param.pattern.span().start;
        let words = self.words(start, pattern);
        if modifiers_allowed {
            self.check_modifier_list(&words, PARAMETER_MODIFIERS, pattern);
            return;
        }
        // Read as a binding: an access modifier is a reserved word, any
        // other modifier is the parameter's name and the real name
        // after it is unexpected.
        if let Some(&(word, at)) = words.first() {
            if ACCESS_MODIFIERS.contains(&word) {
                self.error(at, format!("The keyword '{word}' is reserved"));
            } else {
                let after = words.get(1).map_or(pattern, |&(_, next)| next);
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
        modifiers_allowed: bool,
    ) {
        for param in &params.items {
            self.check_parameter_modifiers(param, modifiers_allowed);
        }
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

/// acorn's wording (V8's) for why a regular expression pattern is
/// invalid, from oxc's.
fn regexp_reason(message: &str) -> String {
    let reason = message
        .strip_prefix("Invalid regular expression:")
        .unwrap_or(message)
        .trim();
    let acorn = match reason {
        "Unterminated capturing group"
        | "Unterminated ignore group"
        | "Unterminated lookaround assertion" => "Unterminated group",
        "Unterminated character class"
        | "Unterminated nested class"
        | "Unterminated class string disjunction" => "Unterminated character class",
        "Unterminated capturing group name" | "Group specifier is empty" => {
            "Invalid capture group name"
        }
        "Duplicated capturing group names" => "Duplicate capture group name",
        "Invalid named reference" => "Invalid named capture referenced",
        "Character class range out of order" => "Range out of order in character class",
        "Numbers out of order in braced quantifier" => "numbers out of order in {} quantifier",
        "Invalid unicode escape sequence" => "Invalid unicode escape",
        "Invalid extended atom escape" | "Invalid indexed reference" => "Invalid escape",
        "Could not parse the entire pattern" => "Unmatched ')'",
        _ if reason.starts_with("Lone quantifier") => "Nothing to repeat",
        _ if reason.starts_with("Invalid unicode property") => "Invalid property name",
        _ => reason,
    };
    acorn.to_string()
}

/// The binding identifiers of a pattern, in source order.
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

/// The first token at or after `at`: whitespace and comments skipped.
pub(crate) fn next_token_start(text: &str, mut at: u32) -> u32 {
    loop {
        let Some(rest) = text.get(at as usize..) else {
            return at;
        };
        let trimmed = rest.trim_start();
        at += (rest.len() - trimmed.len()) as u32;
        if let Some(line) = trimmed.strip_prefix("//") {
            let len = line
                .find(['\n', '\r', '\u{2028}', '\u{2029}'])
                .unwrap_or(line.len());
            at += 2 + len as u32;
        } else if let Some(block) = trimmed.strip_prefix("/*") {
            match block.find("*/") {
                Some(end) => at += 2 + end as u32 + 2,
                None => return text.len() as u32,
            }
        } else {
            return at;
        }
    }
}

impl<'a> Visit<'a> for Checker<'_, 'a> {
    fn visit_statement(&mut self, it: &Statement<'a>) {
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
        if it.declare && self.mode.transpiled {
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
        if it.declare && self.mode.transpiled {
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
        self.with_context(ctx, FUNCTION, |this| {
            this.function_parts(&it.params, it.body.as_deref(), it.span.start, false);
        });
        if it.body.is_none() {
            self.leak_signature_scope(it);
        }
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
        self.with_context(ctx, FUNCTION, |this| match &it.body {
            ArrowFunctionBody::FunctionBody(body) => {
                this.function_parts(&it.params, Some(body), it.span.start, false);
            }
            body => {
                this.function_parts(&it.params, None, it.span.start, false);
                if let Some(expression) = body.as_expression() {
                    this.visit_expression(expression);
                }
            }
        });
    }

    fn visit_formal_parameter(&mut self, it: &FormalParameter<'a>) {
        for decorator in &it.decorators {
            self.visit_decorator(decorator);
        }
        self.visit_binding_pattern(&it.pattern);
        if let Some(init) = &it.initializer {
            self.visit_expression(init);
        }
    }

    fn visit_class(&mut self, it: &Class<'a>) {
        if it.declare && self.mode.transpiled {
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
        for element in &it.body.body {
            if self.settled(element.span().start) {
                break;
            }
            self.class_element(element, derived, &mut had_constructor);
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
        self.with_loop_label(LabelKind::Loop, |this| this.visit_statement(&it.body));
    }

    fn visit_do_while_statement(&mut self, it: &DoWhileStatement<'a>) {
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
        if it.import_kind == ImportOrExportKind::Type && self.mode.transpiled {
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
                    if s.import_kind == ImportOrExportKind::Type && self.mode.transpiled {
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
        walk::walk_unary_expression(self, it);
    }

    fn visit_numeric_literal(&mut self, it: &NumericLiteral<'a>) {
        // TypeScript's transpile prints legacy octal literals anew.
        if self.mode.transpiled {
            return;
        }
        let raw = self.span_text(it.span).as_bytes();
        if raw.len() >= 2 && raw[0] == b'0' && raw[1].is_ascii_digit() {
            self.error(it.span.start, "Invalid number");
        }
    }

    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.check_string_escapes(it.span);
    }

    fn visit_reg_exp_literal(&mut self, it: &RegExpLiteral<'a>) {
        let raw = self.span_text(it.span);
        let Some(close) = raw.rfind('/').filter(|&i| i > 0) else {
            return;
        };
        let (pattern, flags) = (&raw[1..close], &raw[close + 1..]);
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_regular_expression::LiteralParser::new(
            &allocator,
            pattern,
            Some(flags),
            oxc_regular_expression::Options::default(),
        )
        .parse();
        if let Err(error) = parsed {
            let reason = regexp_reason(&error.message);
            let at = it.span.start + 1;
            self.record(
                Candidate::at(
                    at,
                    format!("Invalid regular expression: /{pattern}/: {reason}"),
                )
                .in_token(it.span.start),
            );
        }
    }

    fn visit_super(&mut self, it: &Super) {
        if !self.context().allow_super {
            self.error(it.span.start, "'super' keyword outside a method");
        }
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Super(s) = &it.callee {
            if !self.context().allow_direct_super {
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
        if ctx.static_block {
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
        if crate::transpile_sensitive::tag_has_optional_chain(&it.tag) {
            self.error(
                it.quasi.span.start,
                "Optional chaining cannot appear in the tag of tagged template expressions",
            );
        }
        walk::walk_tagged_template_expression(self, it);
    }

    fn visit_private_in_expression(&mut self, it: &PrivateInExpression<'a>) {
        self.use_private(it.left.name.as_str(), it.left.span.start);
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

    fn visit_ts_interface_declaration(&mut self, it: &TSInterfaceDeclaration<'a>) {
        if self.ts_as_written() {
            self.declare(it.id.name.as_str(), Bind::TsInterface, it.id.span.start);
        }
    }

    fn visit_ts_type_alias_declaration(&mut self, it: &TSTypeAliasDeclaration<'a>) {
        if self.ts_as_written() {
            self.declare(it.id.name.as_str(), Bind::TsType, it.id.span.start);
        }
    }

    fn visit_ts_enum_declaration(&mut self, it: &TSEnumDeclaration<'a>) {
        // acorn-typescript declares nothing for an enum.
        if self.mode.transpiled && !it.declare {
            self.transpiled_var(it.id.name.as_str(), it.id.span.start);
        }
    }

    fn visit_ts_namespace_declaration(&mut self, it: &TSNamespaceDeclaration<'a>) {
        if self.mode.transpiled {
            if !crate::rules::transpile_positions::namespace_removed(it) {
                self.transpiled_var(it.id.name.as_str(), it.id.span.start);
                self.namespace_body(&it.body);
            }
            return;
        }
        // acorn-typescript declares a namespace like a `var`.
        self.declare(it.id.name.as_str(), Bind::Var, it.id.span.start);
        self.namespace_body(&it.body);
    }

    fn visit_ts_external_module_declaration(&mut self, it: &TSExternalModuleDeclaration<'a>) {
        if self.mode.transpiled {
            return;
        }
        if let Some(body) = &it.body {
            self.push_scope(0);
            self.visit_statements(&body.body);
            self.pop_scope();
        }
    }

    fn visit_ts_global_declaration(&mut self, it: &TSGlobalDeclaration<'a>) {
        if self.mode.transpiled {
            return;
        }
        self.push_scope(0);
        self.visit_statements(&it.body.body);
        self.pop_scope();
    }

    fn visit_ts_import_equals_declaration(&mut self, it: &TSImportEqualsDeclaration<'a>) {
        if it.import_kind == ImportOrExportKind::Type && self.mode.transpiled {
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
        let modifiers_allowed = self.mode.ts;
        self.with_context(ctx, FUNCTION, |this| {
            this.function_parts(
                &f.params,
                f.body.as_deref(),
                f.span.start,
                modifiers_allowed,
            );
        });
        if f.body.is_none() {
            self.leak_signature_scope(f);
        }
    }

    /// acorn-typescript (as bundled with the compiler) enters a
    /// function's scope before it knows the function has no body, and
    /// never leaves it for an overload signature, a `declare function`
    /// or an abstract method. Everything after it is read one scope
    /// deeper: declared there, as if inside a plain function, until the
    /// enclosing block's end closes this scope instead of its own.
    fn leak_signature_scope(&mut self, f: &Function<'a>) {
        if !self.ts_as_written() {
            return;
        }
        let labels = self
            .context()
            .labels
            .iter()
            .map(|l| Label {
                name: l.name,
                kind: l.kind,
            })
            .collect();
        self.scopes.push(Scope {
            flags: FUNCTION,
            context: Some(Context {
                function: true,
                is_async: f.r#async,
                generator: f.generator,
                labels,
                ..Context::default()
            }),
            ..Scope::default()
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
        let mode = Mode {
            ts,
            transpiled: false,
        };
        first_early_error(&parsed.program, text, mode).map(|c| {
            let at = match c.at {
                Position::At(at) | Position::ColumnOf(at) => at,
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
    fn typescript_declarations() {
        let ts = |t| first(t, true);
        assert_eq!(
            ts("function f(a: string): void;\nfunction f(a: any) {}"),
            None
        );
        assert_eq!(ts("interface A {}\nclass A {}"), None);
        assert_eq!(ts("type A = 1;\nconst A = 1;"), None);
        assert_eq!(
            ts("type A = 1;\ntype A = 2;"),
            Some((17, "type 'A' has already been declared.".into()))
        );
        assert_eq!(
            ts("function N() {}\nnamespace N { export type X = 1; }"),
            Some((26, "Identifier 'N' has already been declared".into()))
        );
        assert_eq!(
            ts("declare let x: number;\nlet x = 1;"),
            Some((27, "Identifier 'x' has already been declared".into()))
        );
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
        assert_eq!(js("let o = 017;"), Some((8, "Invalid number".into())));
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
    fn modifiers_acorn_typescript_rejects() {
        let ts = |t| first(t, true);
        assert_eq!(
            ts("function f(private a) {}"),
            Some((11, "The keyword 'private' is reserved".into()))
        );
        assert_eq!(
            ts("function f(readonly a) {}"),
            Some((20, "Unexpected token".into()))
        );
        assert_eq!(ts("class K { constructor(private a) {} }"), None);
        assert_eq!(
            ts("class K { static accessor x = 1 }"),
            Some((
                17,
                "'accessor' modifier cannot be used with 'static' modifier.".into()
            ))
        );
        assert_eq!(
            ts("class K { public public x = 1 }"),
            Some((24, "Accessibility modifier already seen.".into()))
        );
        assert_eq!(ts("using x = null;"), Some((6, "Unexpected token".into())));
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
