//! `js_parse_error` for `<script>` contents.
//!
//! The compiler's `parse()` reads each top-level `<script>` with acorn
//! (acorn-typescript in a TypeScript component) as it reaches it, as a
//! strict-mode module. The first syntax error it meets throws
//! `js_parse_error` with acorn's message, ahead of everything the
//! analysis would report. svelte-check still type-checks the component:
//! `svelte2tsx` does not parse scripts with acorn, so the TypeScript
//! diagnostics come through beside the compiler error.
//!
//! The compiler sees each script after the project's preprocessors ran:
//! a transpiled `<script lang="ts">` reaches it as TypeScript's output,
//! and positions in it are mapped back through the source map.
//!
//! Where oxc's parser reports an error, it is translated to acorn's
//! message and position; the checks oxc's parser leaves to its semantic
//! pass, and the places where acorn is stricter, are replayed on the
//! AST (`acorn_early_errors`). The error the parser raises first wins.
//!
//! svelte-check then decorates the message (`createParserErrorDiagnostic`
//! in the language server): a message containing "expected" is dropped
//! altogether when the project has no preprocessor of its own and the
//! component has a `lang`/`type` attribute on a script, style or
//! template tag (it is taken to come from syntax some preprocessor would
//! have handled), and otherwise gets a hint about preprocessing when it
//! lies in the script tag.

use oxc_ast::ast::{
    ArrowFunctionExpression, Expression, LogicalExpression, LogicalOperator, Program,
    VariableDeclaration, VariableDeclarationKind,
};
use oxc_ast_visit::{Visit, walk};
use oxc_diagnostics::OxcDiagnostic;
use oxc_span::GetSpan;
use svn_core::Range;
use svn_parser::{Document, ScriptSection};

use crate::rules::acorn_early_errors::{
    Candidate, Mode, Position, first_early_error, next_token_start,
};

/// The hint svelte-check appends to a parse error inside the script
/// tag whose message contains "expected".
const PREPROCESS_HINT: &str = "\n\nIf you expect this syntax to work, here are some suggestions: \nIf you use typescript with `svelte-preprocess`, did you add `lang=\"ts\"` to your `script` tag? \nDid you setup a `svelte.config.js` or `vite.config.js`? \nSee https://github.com/sveltejs/language-tools/tree/master/docs#using-with-preprocessors for more info.";

/// One script as the lint pass holds it.
pub(crate) struct Script<'d, 'p, 'a> {
    pub section: &'d ScriptSection<'d>,
    pub program: &'p Program<'a>,
    pub errors: &'p [OxcDiagnostic],
    /// oxc gave up on the script: `program` holds nothing.
    pub panicked: bool,
}

/// The project settings the diagnostic depends on.
pub(crate) struct Settings {
    /// The component is TypeScript: every script is read by
    /// acorn-typescript.
    pub ts: bool,
    /// The preprocessors transpile `<script lang="ts">`.
    pub preprocess_ts: bool,
    /// The project's Svelte config brings its own `preprocess`.
    pub preprocess_configured: bool,
}

/// The `js_parse_error` svelte-check reports for the component's
/// scripts, if any: the message (docs link and hint included) and the
/// offset it points at.
pub(crate) fn script_parse_error(
    doc: &Document<'_>,
    source: &str,
    scripts: &[Script<'_, '_, '_>],
    settings: &Settings,
) -> Option<(String, Range)> {
    let mut ordered: Vec<&Script<'_, '_, '_>> = scripts.iter().collect();
    ordered.sort_by_key(|s| s.section.content_range.start);
    for script in ordered {
        let transpiled = crate::rules::typescript_features::script_is_transpiled(
            script.section,
            settings.preprocess_ts,
        );
        let mode = Mode {
            ts: settings.ts,
            transpiled,
        };
        let Some(candidate) = first_error(script, mode) else {
            continue;
        };
        let base = script.section.content_range.start;
        let mut at = match candidate.at {
            Position::At(rel) => base + candidate.token_start.filter(|_| transpiled).unwrap_or(rel),
            Position::ColumnOf(rel) => column_as_offset(source, base + rel),
        };
        if transpiled
            && let Some(mapped) =
                crate::rules::transpile_positions::transpiled_position(base, script.program, at)
        {
            at = mapped;
        }
        let mut message = format!("{}\nhttps://svelte.dev/e/js_parse_error", candidate.message);
        if message.contains("expected") {
            if !settings.preprocess_configured && has_language_attribute(doc) {
                return None;
            }
            if in_script_tag(doc, at) {
                message.push_str(PREPROCESS_HINT);
            }
        }
        return Some((message, Range::new(at, at)));
    }
    None
}

/// A transpiled script the preprocessor runs on past its close tag: the
/// open-tag range of that script, and the error the compiler raises
/// after its body.
///
/// The compiler's `preprocess` finds scripts with a regular expression
/// that needs a literal `</script>` (`regex_script_tags`), so a script
/// the Svelte parser closes some other way (`</script >`) runs on to the
/// next literal close tag, and the preprocessor transpiles all of it,
/// later script blocks included, as one `<script lang="ts">` body. The
/// compiler then reads only that one script. In it, the swallowed
/// `</script …>` is TypeScript's `<` followed by a regular expression
/// from the `/`; unterminated, it survives the transpile and acorn stops
/// there with "Unterminated regular expression", mapped back to the `/`.
/// An error in the script's own body comes first.
pub(crate) fn merged_script_error(
    doc: &Document<'_>,
    source: &str,
    settings: &Settings,
) -> Option<(Range, Option<(String, Range)>)> {
    let spans = svn_parser::script_tag_expression_spans(source);
    let first = [doc.module_script.as_ref(), doc.instance_script.as_ref()]
        .into_iter()
        .flatten()
        .filter(|s| {
            crate::rules::typescript_features::script_is_transpiled(s, settings.preprocess_ts)
        })
        .find(|s| {
            spans.iter().any(|&(start, end)| {
                start == s.open_tag_range.start as usize && end > s.close_tag_range.end as usize
            })
        })?;
    let slash = first.close_tag_range.start as usize + 1;
    let regex_end = svn_parser::typescript_regex_end(source, slash);
    let unterminated = matches!(source.as_bytes().get(regex_end), None | Some(b'\n' | b'\r'));
    let error = unterminated.then(|| {
        (
            "Unterminated regular expression\nhttps://svelte.dev/e/js_parse_error".to_string(),
            Range::new(slash as u32, slash as u32),
        )
    });
    Some((first.open_tag_range, error))
}

/// The first error acorn raises in one script.
fn first_error(script: &Script<'_, '_, '_>, mode: Mode) -> Option<Candidate> {
    let text = script.section.content;
    let from_oxc = script
        .errors
        .first()
        .and_then(|first| translate(first, script.errors, script.program, text, mode))
        .filter(|c| !(mode.transpiled && TRANSPILE_REWRITES.contains(&c.message.as_str())));
    // acorn-typescript reads everything after a bodiless function
    // declaration as inside a function (see `acorn_early_errors`).
    let from_oxc = from_oxc.filter(|c| {
        let allowed_in_function = c.message == "'return' outside of function"
            || c.message.starts_with("'new.target' can only be used");
        !(allowed_in_function
            && crate::rules::acorn_early_errors::signature_scope_start(script.program, mode)
                .is_some_and(|start| start <= c.detect))
    });
    let early = if script.panicked {
        let stop = script
            .errors
            .first()
            .and_then(|d| d.labels.first())
            .map_or(0, |l| l.span().start);
        let before = early_error_before(text, stop, mode);
        let same_line = early_error_in_typescript_reading(text, stop, mode);
        match (before, same_line) {
            (Some(a), Some(b)) => Some(if b.detect < a.detect { b } else { a }),
            (a, b) => a.or(b),
        }
    } else {
        first_early_error(script.program, text, mode)
    };
    match (from_oxc, early) {
        (Some(a), Some(b)) => Some(if b.detect < a.detect { b } else { a }),
        (a, b) => a.or(b),
    }
}

/// Errors TypeScript's transpile repairs by printing the construct anew.
const TRANSPILE_REWRITES: &[&str] = &[
    "Numeric separator must be exactly one underscore",
    "Comma is not permitted after the rest element",
    "Cannot use new with import()",
];

/// How many cut points [`early_error_before`] tries.
const PREFIX_ATTEMPTS: usize = 64;

/// The early errors before `stop` in a script oxc gave up on (oxc then
/// keeps no AST at all). acorn reads the text before its first syntax
/// error, so the checks run on the longest prefix ending at a line
/// start that oxc can parse.
fn early_error_before(text: &str, stop: u32, mode: Mode) -> Option<Candidate> {
    let stop = (stop as usize).min(text.len());
    let cuts = text[..stop]
        .match_indices('\n')
        .map(|(i, _)| i + 1)
        .collect::<Vec<_>>();
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(mode.ts);
    for cut in cuts.into_iter().rev().take(PREFIX_ATTEMPTS) {
        let allocator = oxc_allocator::Allocator::default();
        let prefix = &text[..cut];
        let parsed = oxc_parser::Parser::new(&allocator, prefix, source_type).parse();
        if parsed.panicked {
            continue;
        }
        return first_early_error(&parsed.program, prefix, mode);
    }
    None
}

/// The early errors before `stop` in a JavaScript script oxc gave up on
/// at TypeScript syntax, read from the script's TypeScript parse.
///
/// [`early_error_before`] only sees whole lines before the syntax
/// error, but acorn raises an early error wherever it meets one: in
/// `class K { accessor x: number = 1 }` it takes `accessor` for the
/// field's name and stops at `x`, before the type annotation oxc gave
/// up on. TypeScript is a superset of the JavaScript acorn read up to
/// that point, so when the TypeScript parse succeeds, the JavaScript
/// checks run on its tree find what acorn found before `stop`.
fn early_error_in_typescript_reading(text: &str, stop: u32, mode: Mode) -> Option<Candidate> {
    if mode.ts {
        return None;
    }
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(true);
    let parsed = oxc_parser::Parser::new(&allocator, text, source_type).parse();
    if parsed.panicked || !parsed.diagnostics.is_empty() {
        return None;
    }
    first_early_error(&parsed.program, text, mode).filter(|c| c.detect < stop)
}

/// acorn's version of the first error oxc's parser reported. `None`
/// when acorn accepts what oxc rejects, or when the AST checks place
/// the error themselves.
fn translate(
    diagnostic: &OxcDiagnostic,
    all: &[OxcDiagnostic],
    program: &Program<'_>,
    text: &str,
    mode: Mode,
) -> Option<Candidate> {
    let message = diagnostic.message.as_ref();
    let span = diagnostic.labels.first().map(|l| l.span())?;
    let (s, e) = (span.start, span.end);
    let unexpected = |at: u32| Some(Candidate::at(at, "Unexpected token"));
    let char_at = |at: u32| text.get(at as usize..).and_then(|r| r.chars().next());
    let ts = mode.ts;

    // Messages oxc and acorn share.
    const SAME: &[&str] = &[
        "Constructor can't be a generator",
        "Constructor can't have get/set modifier",
        "Bad escape sequence in untagged template literal",
    ];
    if SAME.contains(&message) {
        return Some(Candidate::at(s, message));
    }
    // Messages that only change wording.
    const RENAMED: &[(&str, &str)] = &[
        ("Unterminated string", "Unterminated string constant"),
        (
            "`await` is only allowed within async functions and at the top levels of modules",
            "Cannot use keyword 'await' outside an async function",
        ),
        (
            "Cannot use `yield` as an identifier in a generator context",
            "Cannot use 'yield' as identifier inside a generator",
        ),
        (
            "A 'return' statement can only be used within a function body.",
            "'return' outside of function",
        ),
        (
            "Unexpected new.target expression",
            "'new.target' can only be used in functions and class static block",
        ),
        (
            "A 'get' accessor must not have any formal parameters.",
            "getter should have no params",
        ),
        (
            "A 'set' accessor must have exactly one parameter.",
            "setter should have exactly one param",
        ),
        (
            "Invalid characters after number",
            "Identifier directly after number",
        ),
        (
            "A rest parameter or binding pattern may not have a trailing comma.",
            "Comma is not permitted after the rest element",
        ),
        (
            "Tagged template expressions are not permitted in an optional chain",
            "Optional chaining cannot appear in the tag of tagged template expressions",
        ),
        (
            "Classes may not have a static property named 'prototype'",
            "Classes can't have a static field named 'prototype'",
        ),
    ];
    if let Some((_, acorn)) = RENAMED.iter().find(|(oxc, _)| *oxc == message) {
        if message == "Unterminated string" && char_at(s) == Some('`') {
            return Some(Candidate::at(s + 1, "Unterminated template").in_token(s));
        }
        if message == "Invalid characters after number" {
            return Some(Candidate::at(s, *acorn).in_token(number_start(text, s)));
        }
        return Some(Candidate::at(s, *acorn));
    }

    match message {
        "Unexpected token" | "Unexpected JSX expression" => {
            if word_at(text, s) == "enum" {
                return Some(Candidate::at(s, "The keyword 'enum' is reserved"));
            }
            return unexpected(s);
        }
        "Cannot use `await` as an identifier in an async context" => {
            // An async arrow's parameters are first read as call
            // arguments, where `await` starts an await expression that
            // then lacks its operand.
            if in_async_arrow_params(program, s) {
                return unexpected(next_token_start(text, identifier_end(text, s)));
            }
            return Some(Candidate::at(
                s,
                "Cannot use keyword 'await' outside an async function",
            ));
        }
        "Unterminated regular expression" => {
            return Some(Candidate::at(s + 1, "Unterminated regular expression").in_token(s));
        }
        "Lexical declaration cannot appear in a single-statement context" => {
            // `let` there is read as an identifier, `const` as nothing.
            if word_at(text, s) == "let" {
                return Some(Candidate::at(s, "The keyword 'let' is reserved"));
            }
            return unexpected(s);
        }
        "Declaration cannot appear in a single-statement context" => return unexpected(s),
        "Cannot assign to this expression" => {
            let target = text.get(s as usize..e as usize).unwrap_or_default();
            let message = if target.contains("?.") {
                "Optional chaining cannot appear in left-hand side"
            } else {
                "Assigning to rvalue"
            };
            return Some(Candidate::at(s, message));
        }
        "HTML comments are not allowed in modules" => {
            // acorn-typescript reads `<` as the start of a type
            // argument list and fails one character later.
            return unexpected(if ts { s + 1 } else { s });
        }
        "Line terminator not permitted before arrow" => {
            // Empty parentheses fail at their `)`; anything else was a
            // complete expression and fails at the arrow.
            let before = text[..s as usize].trim_end();
            if before.ends_with(')') && before[..before.len() - 1].trim_end().ends_with('(') {
                return unexpected((before.len() - 1) as u32);
            }
            return unexpected(s);
        }
        "Invalid assignment in object literal" => {
            // TypeScript's transpile maps the `=` from where the name
            // ends.
            let name_end = identifier_end(text, s);
            return Some(
                Candidate::at(
                    next_token_start(text, name_end),
                    "Shorthand property assignments are valid only in destructuring patterns",
                )
                .in_token(name_end),
            );
        }
        "A rest parameter cannot have an initializer." => {
            return unexpected(next_token_start(text, identifier_end(text, s)));
        }
        "Missing initializer in const declaration" => {
            let end = declarator_at(program, s).map_or(e, |(end, _)| end);
            return unexpected(next_token_start(text, end));
        }
        "Missing initializer in destructuring declaration" => {
            // `const` fails at the token after the binding; `let` / `var`
            // once the binding has been read.
            return match declarator_at(program, s) {
                Some((end, VariableDeclarationKind::Const)) => {
                    unexpected(next_token_start(text, end))
                }
                Some((end, _)) => Some(Candidate::at(
                    end,
                    "Complex binding patterns require an initialization value",
                )),
                None => Some(Candidate::at(
                    e,
                    "Complex binding patterns require an initialization value",
                )),
            };
        }
        "Expected function body" => {
            // The label runs to the end of the parameter list; acorn
            // wants the body's `{` right there.
            return unexpected(next_token_start(text, e));
        }
        "Empty parenthesized expression" => {
            return unexpected(next_token_start(text, s + 1));
        }
        "Cannot use new with dynamic import" => {
            if ts {
                return Some(Candidate::at(s, "Cannot use new with import()"));
            }
            return unexpected(next_token_start(text, s + "import".len() as u32));
        }
        "Decorators are not valid here." => {
            return (!ts).then(|| Candidate::at(s, "Unexpected character '@'"));
        }
        "The 'u' and 'v' regular expression flags cannot be enabled at the same time" => {
            return Some(Candidate::at(s + 1, "Invalid regular expression flag").in_token(s));
        }
        "Invalid escape sequence" => return escape_error(text, s),
        _ => {}
    }

    if message.starts_with("Expected a semicolon or an implicit semicolon") {
        // A strict-mode reserved word read as an identifier fails
        // before the statement does.
        let before = text[..s as usize].trim_end();
        let word_start = before
            .char_indices()
            .rev()
            .take_while(|&(_, c)| oxc_syntax::identifier::is_identifier_part(c))
            .last()
            .map_or(before.len(), |(i, _)| i);
        let word = &before[word_start..];
        if RESERVED_IN_STRICT_MODE.contains(&word)
            && !before[..word_start].trim_end().ends_with(['.', '?'])
        {
            return Some(Candidate::at(
                word_start as u32,
                format!("The keyword '{word}' is reserved"),
            ));
        }
        return unexpected(next_token_start(text, s));
    }
    if message.starts_with("Expected `") && message.contains("but found") {
        return unexpected(s);
    }
    if let Some(rest) = message.strip_prefix("Identifier expected. '") {
        let word = rest.split('\'').next().unwrap_or_default();
        let message = if word == "enum" {
            format!("The keyword '{word}' is reserved")
        } else {
            format!("Unexpected keyword '{word}'")
        };
        return Some(Candidate::at(s, message));
    }
    if let Some(rest) = message.strip_prefix("Duplicated export '") {
        // acorn-typescript does not track exports.
        if ts {
            return None;
        }
        let name = rest.split('\'').next().unwrap_or_default();
        let at = diagnostic.labels.last().map_or(s, |l| l.span().start);
        return Some(Candidate::at(at, format!("Duplicate export '{name}'")));
    }
    if let Some(rest) = message.strip_prefix("Invalid Character `") {
        let c = rest.chars().next().unwrap_or_default();
        if char_at(s) != Some(c) {
            // The character came from a `\u` escape in an identifier.
            let escape = text[..s as usize].rfind("\\u").map_or(s, |i| i as u32);
            return Some(Candidate::at(escape, "Invalid Unicode escape"));
        }
        if c == '_' && s > 0 && text.as_bytes().get(s as usize - 1) == Some(&b'_') {
            return Some(Candidate::at(
                s,
                "Numeric separator must be exactly one underscore",
            ));
        }
        return Some(Candidate::at(s, format!("Unexpected character '{c}'")));
    }
    if message.starts_with("Flag ") && message.contains("is mentioned twice") {
        // The regular expression's start comes with the next report.
        let start = all
            .iter()
            .skip(1)
            .find(|d| d.message == "Unexpected token")
            .and_then(|d| d.labels.first().map(|l| l.span().start))?;
        return Some(Candidate::at(start + 1, "Duplicate regular expression flag").in_token(start));
    }
    if message.starts_with("A unary expression with the") {
        return unexpected(next_token_start(text, e));
    }
    if message.starts_with("Logical expressions and coalesce expressions cannot be mixed") {
        let (at, operand_end) = mixed_operator(program, text, s)?;
        // TypeScript's transpile maps an operator from where the operand
        // before it ends.
        return Some(
            Candidate::at(
                at,
                "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses",
            )
            .in_token(operand_end),
        );
    }
    if message.starts_with('\'') && message.ends_with("' modifier cannot be used here.") {
        // The modifier is read as an identifier and what follows it is
        // unexpected.
        return unexpected(next_token_start(text, identifier_end(text, s)));
    }
    // TypeScript-only syntax in a JavaScript component: acorn fails at
    // the token oxc reports, except for constructs the AST checks place.
    if diagnostic
        .code
        .number
        .as_deref()
        .is_some_and(|n| n.starts_with('8'))
    {
        return (!ts
            && !message.starts_with("Parameter modifiers")
            && !message.starts_with("Type assertion expressions")
            && !message.starts_with("Type satisfaction expressions")
            && !message.starts_with("'implements' clauses"))
        .then(|| Candidate::at(s, "Unexpected token"));
    }
    // Other checks TypeScript defines: acorn-typescript is more lenient.
    if diagnostic.code.is_some() {
        return None;
    }
    unexpected(s)
}

/// Words strict-mode code may not use as identifiers.
const RESERVED_IN_STRICT_MODE: &[&str] = &[
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

/// The start of the numeric literal whose text runs up to `at`.
fn number_start(text: &str, at: u32) -> u32 {
    let before = &text[..at as usize];
    let len = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_')
        .count();
    at - len as u32
}

/// acorn's errors for a malformed `\u` / `\x` escape at `at`.
fn escape_error(text: &str, at: u32) -> Option<Candidate> {
    let rest = text.get(at as usize..)?;
    if let Some(body) = rest.strip_prefix("\\u{") {
        let digits = body.split('}').next().unwrap_or_default();
        let message = match u32::from_str_radix(digits, 16) {
            Ok(code) if code > 0x10FFFF => "Code point out of bounds",
            _ => "Bad character escape sequence",
        };
        return Some(Candidate::at(at + 3, message));
    }
    if rest.starts_with("\\u") || rest.starts_with("\\x") {
        return Some(Candidate::at(at + 2, "Bad character escape sequence"));
    }
    None
}

/// The word (identifier characters) starting at `at`.
fn word_at(text: &str, at: u32) -> &str {
    let end = identifier_end(text, at);
    text.get(at as usize..end as usize).unwrap_or_default()
}

/// The end of the identifier starting at `at`.
fn identifier_end(text: &str, at: u32) -> u32 {
    let rest = text.get(at as usize..).unwrap_or_default();
    let len = rest
        .char_indices()
        .find(|&(_, c)| !oxc_syntax::identifier::is_identifier_part(c))
        .map_or(rest.len(), |(i, _)| i);
    at + len as u32
}

/// Whether `at` lies in the parameter list of an async arrow function.
fn in_async_arrow_params(program: &Program<'_>, at: u32) -> bool {
    struct Finder {
        at: u32,
        found: bool,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
            if it.r#async && it.params.span.start <= self.at && self.at < it.params.span.end {
                self.found = true;
            }
            walk::walk_arrow_function_expression(self, it);
        }
    }
    let mut finder = Finder { at, found: false };
    finder.visit_program(program);
    finder.found
}

/// The end of the variable declarator whose binding starts at `at`,
/// and the kind of its declaration.
fn declarator_at(program: &Program<'_>, at: u32) -> Option<(u32, VariableDeclarationKind)> {
    struct Finder {
        at: u32,
        found: Option<(u32, VariableDeclarationKind)>,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
            for declarator in &it.declarations {
                if declarator.id.span().start == self.at {
                    self.found = Some((declarator.span.end, it.kind));
                }
            }
            walk::walk_variable_declaration(self, it);
        }
    }
    let mut finder = Finder { at, found: None };
    finder.visit_program(program);
    finder.found
}

/// In a chain of `&&` / `||` / `??` starting at `at`, the operator
/// where the other kind first appears — where acorn stops.
fn mixed_operator(program: &Program<'_>, text: &str, at: u32) -> Option<(u32, u32)> {
    /// The chain's operators in source order (with where the operand
    /// before each ends), and whether each is `??`.
    fn collect(l: &LogicalExpression<'_>, text: &str, out: &mut Vec<((u32, u32), bool)>) {
        if let Expression::LogicalExpression(left) = &l.left {
            collect(left, text, out);
        }
        let end = l.left.span().end;
        out.push((
            (next_token_start(text, end), end),
            l.operator == LogicalOperator::Coalesce,
        ));
        if let Expression::LogicalExpression(right) = &l.right {
            collect(right, text, out);
        }
    }
    struct Finder<'t> {
        text: &'t str,
        at: u32,
        operators: Option<Vec<((u32, u32), bool)>>,
    }
    impl<'a> Visit<'a> for Finder<'_> {
        fn visit_logical_expression(&mut self, it: &LogicalExpression<'a>) {
            if self.operators.is_none() && it.span.start == self.at {
                let mut out = Vec::new();
                collect(it, self.text, &mut out);
                self.operators = Some(out);
                return;
            }
            walk::walk_logical_expression(self, it);
        }
    }
    let mut finder = Finder {
        text,
        at,
        operators: None,
    };
    finder.visit_program(program);
    let operators = finder.operators?;
    let first = operators.first()?.1;
    operators.iter().find(|(_, c)| *c != first).map(|(p, _)| *p)
}

/// acorn-typescript's column-for-offset slip: the column (in UTF-16
/// units) of the token at `at`, taken as a UTF-16 offset into the file.
fn column_as_offset(source: &str, at: u32) -> u32 {
    let line_start = source[..at as usize].rfind('\n').map_or(0, |i| i + 1);
    let column: usize = source[line_start..at as usize]
        .chars()
        .map(char::len_utf16)
        .sum();
    let mut units = 0;
    for (i, c) in source.char_indices() {
        if units >= column {
            return i as u32;
        }
        units += c.len_utf16();
    }
    source.len() as u32
}

/// svelte-check's `hasLanguageAttribute`: a `lang` or `type` attribute
/// on the script (the instance script's tag, or the module script's
/// when there is no instance script), the style or a template tag.
fn has_language_attribute(doc: &Document<'_>) -> bool {
    let set = |attrs: &[svn_parser::ScriptAttr]| {
        let value = |name: &str| {
            attrs
                .iter()
                .rev()
                .find(|a| a.name == name)
                .and_then(|a| a.value.as_deref())
                .filter(|v| !v.is_empty())
        };
        value("lang")
            .or_else(|| value("type"))
            .is_some_and(|v| !v.strip_prefix("text/").unwrap_or(v).is_empty())
    };
    let script = doc
        .instance_script
        .as_ref()
        .or(doc.module_script.as_ref())
        .is_some_and(|s| set(&s.attrs));
    script || doc.style.as_ref().is_some_and(|s| set(&s.attrs))
}

/// svelte-check's `isInTag` check for the script: the instance script's
/// content (or the module script's when there is no instance script),
/// both ends included.
fn in_script_tag(doc: &Document<'_>, at: u32) -> bool {
    doc.instance_script
        .as_ref()
        .or(doc.module_script.as_ref())
        .is_some_and(|s| s.content_range.start <= at && at <= s.content_range.end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use svn_parser::ScriptLang;

    /// The reported error as `line:column` (0-based) plus the message's
    /// first line, and whether the preprocessing hint was appended.
    fn report(source: &str, preprocess_ts: bool, preprocess_configured: bool) -> Option<String> {
        let (doc, _) = svn_parser::parse_sections(source);
        let ts = crate::rules::typescript_features::compiler_parses_as_ts(source);
        let lang = if ts { ScriptLang::Ts } else { ScriptLang::Js };
        let allocator = Allocator::default();
        let parsed: Vec<_> = [doc.module_script.as_ref(), doc.instance_script.as_ref()]
            .into_iter()
            .flatten()
            .map(|s| {
                (
                    s,
                    svn_parser::parse_script_body(&allocator, s.content, lang),
                )
            })
            .collect();
        let scripts: Vec<Script<'_, '_, '_>> = parsed
            .iter()
            .map(|(section, p)| Script {
                section,
                program: &p.program,
                errors: &p.errors,
                panicked: p.panicked,
            })
            .collect();
        let settings = Settings {
            ts,
            preprocess_ts,
            preprocess_configured,
        };
        let (message, range) = script_parse_error(&doc, source, &scripts, &settings)?;
        let positions = svn_core::PositionMap::new(source);
        let at = positions.position_of(range.start);
        let hint = if message.contains("If you expect") {
            " +hint"
        } else {
            ""
        };
        let first = message.lines().next().unwrap_or_default();
        Some(format!("{}:{} {first}{hint}", at.line, at.character))
    }

    #[test]
    fn optional_chain_as_template_tag() {
        let src = "<script lang=\"ts\">\na?.b`x`;\n</script>\n";
        assert_eq!(
            report(src, true, false).as_deref(),
            Some("1:4 Optional chaining cannot appear in the tag of tagged template expressions")
        );
    }

    fn as_written(source: &str) -> Option<String> {
        report(source, false, true)
    }

    #[test]
    fn syntax_errors_take_acorn_positions() {
        assert_eq!(
            as_written("<script>\nlet a = 1 2;\n</script>").as_deref(),
            Some("1:10 Unexpected token +hint")
        );
        assert_eq!(
            as_written("<script>\ninterface A {}\n</script>").as_deref(),
            Some("1:0 The keyword 'interface' is reserved")
        );
        assert_eq!(
            as_written("<script>\nenum E { A }\n</script>").as_deref(),
            Some("1:0 The keyword 'enum' is reserved")
        );
        assert_eq!(
            as_written("<script>\nlet s = `abc;\n</script>").as_deref(),
            Some("1:9 Unterminated template")
        );
        assert_eq!(
            as_written("<script>\nlet x = a ?? b || c;\n</script>").as_deref(),
            Some(
                "1:15 Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses"
            )
        );
    }

    #[test]
    fn early_error_on_the_line_of_a_typescript_syntax_error() {
        // acorn reads `accessor` as the field name and stops at `x`,
        // before the annotation oxc's JavaScript parse gives up on.
        assert_eq!(
            as_written("<script>\nclass K { accessor x: number = 1 }\n</script>").as_deref(),
            Some("1:19 Unexpected token +hint")
        );
    }

    #[test]
    fn early_error_before_a_later_syntax_error() {
        assert_eq!(
            as_written("<script>\nlet x = 1;\nlet x = 2;\nlet y = ;\n</script>").as_deref(),
            Some("2:4 Identifier 'x' has already been declared")
        );
    }

    #[test]
    fn modifier_errors_slip_to_the_column_offset() {
        assert_eq!(
            as_written("<script lang=\"ts\">\nclass K {\n  static accessor x = 1\n}\n</script>")
                .as_deref(),
            Some("0:9 'accessor' modifier cannot be used with 'static' modifier.")
        );
    }

    #[test]
    fn missing_preprocess_hides_expected_messages() {
        let broken = "<script lang=\"ts\">\nlet v: number = ;\n</script>";
        assert_eq!(report(broken, true, false), None);
        assert_eq!(
            report(broken, false, true).as_deref(),
            Some("1:16 Unexpected token +hint")
        );
        // Other messages are kept, at the start of the token in the
        // transpiled script.
        assert_eq!(
            report(
                "<script lang=\"ts\">\nlet s = /abc;\n</script>",
                true,
                false
            )
            .as_deref(),
            Some("1:8 Unterminated regular expression")
        );
    }

    #[test]
    fn hint_only_inside_the_script_svelte_check_looks_at() {
        // With an instance script, only positions inside it get the hint.
        assert_eq!(
            as_written("<script module>\nlet a = 1 2;\n</script>\n<script>\nlet b = 1;\n</script>")
                .as_deref(),
            Some("1:10 Unexpected token")
        );
    }
}
