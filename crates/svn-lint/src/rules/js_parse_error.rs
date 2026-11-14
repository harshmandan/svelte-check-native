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
use oxc_syntax::scope::ScopeFlags;
use svn_core::Range;
use svn_parser::{Document, ScriptSection};

use crate::rules::acorn_early_errors::{
    Candidate, Mode, Position, first_early_error, next_token_start,
};
use crate::rules::acorn_typescript_attempts::settled_error;

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
    // A JavaScript script the compiler reads as written is read by
    // acorn itself: the port says what acorn says.
    if !mode.ts && !mode.transpiled {
        return svn_acorn::parse(script.section.content, svn_acorn::Reading::Script)
            .err()
            .map(|error| Candidate::at(error.pos, error.message));
    }
    first_error_in(
        script.section.content,
        script.program,
        script.errors,
        script.panicked,
        mode,
    )
}

/// The first error acorn raises reading `text`, given oxc's parse of
/// it: the program, the parser's errors, and whether it gave up.
pub(crate) fn first_error_in(
    text: &str,
    program: &Program<'_>,
    errors: &[OxcDiagnostic],
    panicked: bool,
    mode: Mode,
) -> Option<Candidate> {
    let from_oxc = errors
        .first()
        .and_then(|first| translate(first, errors, program, text, mode))
        .filter(|c| !(mode.transpiled && TRANSPILE_REWRITES.contains(&c.message.as_str())));
    // acorn-typescript reads everything after a bodiless function
    // declaration as inside a function (see `acorn_early_errors`).
    let from_oxc = from_oxc.filter(|c| {
        let allowed_in_function = c.message == "'return' outside of function"
            || c.message.starts_with("'new.target' can only be used");
        !(allowed_in_function
            && crate::rules::acorn_early_errors::signature_scope_start(program, mode)
                .is_some_and(|start| start <= c.detect))
    });
    // Where oxc gave up: nothing it reports lies past that.
    let gave_up_at = errors
        .iter()
        .filter_map(|d| d.labels.first())
        .map(|l| l.span().start)
        .max()
        .unwrap_or(0);
    let early = if panicked {
        let stop = errors
            .first()
            .and_then(|d| d.labels.first())
            .map_or(0, |l| l.span().start);
        let before = early_error_before(text, stop, mode);
        let same_line = early_error_in_typescript_reading(text, stop, mode);
        [
            before,
            same_line,
            early_error_in_completed_prefix(text, gave_up_at, mode),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|c| c.detect)
    } else {
        first_early_error(program, text, mode)
    };
    let found = match (from_oxc, early) {
        (Some(a), Some(b)) => {
            if b.detect < a.detect {
                b
            } else {
                a
            }
        }
        (a, b) => a.or(b)?,
    };
    // acorn-typescript reads an expression that starts with `<` twice,
    // and the error it ends up with is not always this one.
    if !mode.ts || mode.transpiled || !text.contains('<') {
        return Some(found);
    }
    if !panicked {
        return Some(settled_error(text, program, text.len() as u32, mode, found));
    }
    let prefix = text.get(..gave_up_at as usize)?;
    for seed in ["", " 0"] {
        if let Some(completed) = completed_prefix(prefix, seed, mode) {
            let allocator = oxc_allocator::Allocator::default();
            let parsed =
                oxc_parser::Parser::new(&allocator, &completed, source_type_of(mode)).parse();
            return Some(settled_error(
                text,
                &parsed.program,
                gave_up_at,
                mode,
                found,
            ));
        }
    }
    Some(found)
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
        if parsed.fatal_error {
            continue;
        }
        return first_early_error(&parsed.program, prefix, mode);
    }
    None
}

/// How many tokens [`early_error_in_completed_prefix`] adds at most.
const COMPLETION_ATTEMPTS: usize = 16;

/// The early errors acorn raises before `stop`, the token oxc gave up
/// at, in the statement that token is part of.
///
/// acorn raises an early error where it meets one, so `let = ;` is a
/// reserved word at `let`, not an unexpected `;`. [`early_error_before`]
/// sees only the whole lines before the error. Here the text is cut at
/// `stop` and completed into something oxc can parse. Everything before
/// `stop` in that tree is what acorn had read, so the checks run on it
/// find what acorn found there.
fn early_error_in_completed_prefix(text: &str, stop: u32, mode: Mode) -> Option<Candidate> {
    let prefix = text.get(..stop as usize)?;
    // Inside a token the tokenizer failed, and what it read of the
    // token is not text acorn got past.
    let word = |c: Option<char>| c.is_some_and(oxc_syntax::identifier::is_identifier_part);
    if word(prefix.chars().next_back()) && word(text[stop as usize..].chars().next()) {
        return None;
    }
    // acorn-typescript reads a word by what follows it: `interface` and
    // `private` are keywords before some tokens and names before others.
    // The completion follows the last word with something else, so in
    // TypeScript only what lies before that word is read the same way.
    let trusted = if mode.ts {
        let read = prefix.trim_end();
        let last_word = read
            .char_indices()
            .rev()
            .take_while(|&(_, c)| oxc_syntax::identifier::is_identifier_part(c))
            .last()
            .map_or(read.len(), |(i, _)| i);
        last_word as u32
    } else {
        stop
    };
    // The cut may come where an expression is due (`x = `) or where
    // one would be wrong (`f(a) `): try the prefix with and without one.
    ["", " 0"].into_iter().find_map(|expression| {
        let completed = completed_prefix(prefix, expression, mode)?;
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, &completed, source_type_of(mode)).parse();
        first_error_in(
            &completed,
            &parsed.program,
            &parsed.diagnostics,
            false,
            mode,
        )
        .filter(|c| c.detect < trusted)
        // `{ a = 1 }` is an error only once acorn sees no `=` after the
        // object, which is past the cut.
        .filter(|c| !c.message.starts_with("Shorthand property assignments"))
    })
}

fn source_type_of(mode: Mode) -> oxc_span::SourceType {
    oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(mode.ts)
}

/// `prefix` and `seed` followed by the tokens oxc asks for until it
/// parses: a `0` where it wants an expression, and each closing token
/// it names. `None` when that does not get there.
fn completed_prefix(prefix: &str, seed: &str, mode: Mode) -> Option<String> {
    let mut completed = format!("{prefix}{seed}");
    for _ in 0..COMPLETION_ATTEMPTS {
        let allocator = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&allocator, &completed, source_type_of(mode)).parse();
        if !parsed.fatal_error {
            return Some(completed);
        }
        // Where oxc gave up: nothing it reports lies past that.
        let (at, fatal) = parsed
            .diagnostics
            .iter()
            .filter_map(|d| Some((d.labels.first()?.span().start as usize, d)))
            .max_by_key(|&(at, _)| at)?;
        let message = fatal.message.as_ref();
        let wanted = message
            .strip_prefix("Expected `")
            .and_then(|rest| rest.strip_suffix("` but found `EOF`"));
        match wanted {
            Some(token) => {
                completed.push(' ');
                completed.push_str(token);
            }
            // Reported at the function's parameters.
            None if message == "Expected function body" => completed.push_str(" {}"),
            // Inside the prefix it is an error no completion repairs.
            None if message == "Unexpected token" && at >= prefix.len() => {
                completed.push_str(" 0");
            }
            None => return None,
        }
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
    if parsed.fatal_error || !parsed.diagnostics.is_empty() {
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

    if ts && let Some(decided) = acorn_typescript_wording(diagnostic, program, text) {
        return decided;
    }
    if let Some(candidate) = acorn_wording(diagnostic, all, text, mode) {
        return Some(candidate);
    }

    // Messages oxc and acorn share.
    const SAME: &[&str] = &[
        "Classes can't have an element named '#constructor'",
        "Constructor can't be an async method",
        "Classes can't have a field named 'constructor'",
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
        ("Unterminated multiline comment", "Unterminated comment"),
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
        if message == "Unterminated string" && matches!(char_at(s), Some('`' | '}')) {
            let (at, message) = svn_parser::acorn::unterminated_template(text, s, ts);
            return Some(Candidate::at(at, message).in_token(s));
        }
        if message == "Invalid characters after number" {
            // Where acorn reads the number whole, what follows it is a
            // token of its own that nothing allows there.
            return number_error(message, text, s).or_else(|| unexpected(s));
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
            // As a name being declared, inside an async function.
            if in_async_function(program, s) {
                return Some(Candidate::at(
                    s,
                    "Cannot use 'await' as identifier inside an async function",
                ));
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
            // acorn has no `using` declaration: the word is a name, and
            // the name after it has no business there.
            if word_at(text, s) == "using" {
                return unexpected(next_token_start(text, s + 5));
            }
            return unexpected(s);
        }
        "Declaration cannot appear in a single-statement context" => return unexpected(s),
        "Cannot assign to this expression" => {
            // A getter or setter in an object pattern, at its name.
            let before = &text[..s as usize];
            let name = before
                .char_indices()
                .rev()
                .take_while(|&(_, c)| oxc_syntax::identifier::is_identifier_part(c))
                .last()
                .map(|(i, _)| i);
            if let Some(name) = name
                && (before[..name].trim_end().ends_with(" get")
                    || before[..name].trim_end().ends_with(" set")
                    || before[..name].trim_end().ends_with("{get")
                    || before[..name].trim_end().ends_with("{set"))
            {
                return Some(Candidate::at(
                    name as u32,
                    "Object pattern can't contain getter or setter",
                ));
            }
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

    if matches!(
        message,
        "Spread must be last element" | "A rest parameter must be last in a parameter list"
    ) && let Some(comma) = comma_after_rest(text, s)
    {
        return Some(Candidate::at(
            comma,
            "Comma is not permitted after the rest element",
        ));
    }
    if message.starts_with("Expected a semicolon or an implicit semicolon")
        && text[s as usize..].trim_start().starts_with("=>")
        && !in_type_alias(text, s)
        && let Some(decided) = arrow_parameters_error(text, s, ts)
        && (decided.is_some() || ts)
    {
        // Nothing wrong with them: acorn-typescript reads parameters
        // oxc does not (a cast around a name).
        let candidate = decided?;
        // oxc read the parameters as a parenthesized expression and
        // stopped at the arrow; acorn turns that expression into
        // parameters and rejects what cannot be one.
        return Some(candidate);
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
        if ts && word == "interface" && !before[..word_start].trim_end().ends_with(['.', '?']) {
            return Some(Candidate::at(
                next_token_start(text, s),
                "'interface' declarations must be followed by an identifier.",
            ));
        }
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
    if message.starts_with("Expected `(` but found `=`") {
        // A class field named `constructor`, which oxc reads as the
        // constructor method.
        let before = text[..s as usize].trim_end();
        if let Some(start) = before.strip_suffix("constructor").map(str::len)
            && before[..start].trim_end().ends_with(['{', ';', '}'])
        {
            return Some(Candidate::at(
                start as u32,
                "Classes can't have a field named 'constructor'",
            ));
        }
    }
    if message.starts_with("Expected `in` but found") {
        // A private name not followed by `in`. Outside every class acorn
        // stops at the name itself.
        let before = text[..s as usize].trim_end();
        let name_start = before
            .char_indices()
            .rev()
            .take_while(|&(_, c)| oxc_syntax::identifier::is_identifier_part(c))
            .last()
            .map_or(before.len(), |(i, _)| i);
        if let Some(hash) = name_start.checked_sub(1)
            && before.as_bytes()[hash] == b'#'
            && !before[..hash].contains("class")
        {
            return unexpected(hash as u32);
        }
        return unexpected(s);
    }
    if message.starts_with("A unary expression with the")
        && message.ends_with("exponentiation expression")
    {
        // acorn reads the unary expression and then meets the `**`.
        return unexpected(next_token_start(text, e));
    }
    if message.starts_with("Expected `:` but found")
        && let Some(candidate) = keyword_as_shorthand_name(text, s)
    {
        return Some(candidate);
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

/// acorn's keywords: a name it refuses wherever an identifier is read.
const KEYWORDS: &[&str] = &[
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
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
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
];

/// acorn's error for a keyword written as a shorthand property name
/// (`{ class }`, `{ a, case = 1 }`), which oxc reads as a property key
/// and reports at the token after it (`at`). acorn checks the name
/// itself, so its error is at the word.
fn keyword_as_shorthand_name(text: &str, at: u32) -> Option<Candidate> {
    let before = text.get(..at as usize)?.trim_end();
    let word_start = before
        .char_indices()
        .rev()
        .take_while(|&(_, c)| oxc_syntax::identifier::is_identifier_part(c))
        .last()
        .map(|(i, _)| i)?;
    let word = &before[word_start..];
    if !before[..word_start].trim_end().ends_with(['{', ',']) {
        return None;
    }
    let message = match word {
        "await" => "Cannot use keyword 'await' outside an async function".to_string(),
        "enum" => "The keyword 'enum' is reserved".to_string(),
        _ if KEYWORDS.contains(&word) => format!("Unexpected keyword '{word}'"),
        _ => return None,
    };
    Some(Candidate::at(word_start as u32, message))
}

/// The comma that follows the rest element whose `...` is at `rest`.
fn comma_after_rest(text: &str, rest: u32) -> Option<u32> {
    let target = next_token_start(text, rest + 3);
    let end = match text.as_bytes().get(target as usize)? {
        b'{' | b'[' => svn_parser::acorn::match_bracket(text, target).ok()?,
        _ => identifier_end(text, target),
    };
    let comma = next_token_start(text, end);
    (text.as_bytes().get(comma as usize) == Some(&b',')).then_some(comma)
}

/// acorn's error for arrow-function parameters that oxc read as a
/// parenthesized expression ending at `arrow` in `text`.
///
/// acorn reads them as an expression too, then converts each one to a
/// binding (`toAssignable`, then `checkParams`): a spread must come
/// last, a default uses `=`, and the target is a name or a pattern, not
/// a member, a literal or something in parentheses of its own.
fn arrow_parameters_error(text: &str, arrow: u32, ts: bool) -> Option<Option<Candidate>> {
    use oxc_ast::ast::{ArrayExpressionElement, ObjectPropertyKind};

    // The parameters are the last parenthesized group before the arrow.
    let before = text[..arrow as usize].trim_end();
    let open = {
        let mut depth = 0usize;
        let mut found = None;
        for (i, c) in before.char_indices().rev() {
            match c {
                ')' => depth += 1,
                '(' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        found = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        found?
    };
    let parameters = &before[open..];
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(ts);
    let expression = oxc_parser::Parser::new(&allocator, parameters, source_type)
        .parse_expression()
        .ok()?;
    let Expression::ParenthesizedExpression(group) = &expression else {
        return None;
    };

    struct Check<'s> {
        text: &'s str,
        /// acorn-typescript reads the component.
        ts: bool,
        first: Option<(u32, &'static str)>,
    }
    impl Check<'_> {
        fn error(&mut self, at: u32, message: &'static str) {
            if self.first.is_none_or(|(first, _)| at < first) {
                self.first = Some((at, message));
            }
        }

        fn spread(&mut self, span: oxc_span::Span, argument: &Expression<'_>, last: bool) {
            if !last {
                let comma = next_token_start(self.text, span.end);
                self.error(comma, "Comma is not permitted after the rest element");
            }
            if matches!(argument, Expression::AssignmentExpression(_)) {
                self.error(
                    argument.span().start,
                    "Rest elements cannot have a default value",
                );
            }
            self.target(argument);
        }

        fn target(&mut self, expression: &Expression<'_>) {
            match expression {
                Expression::Identifier(_) => {}
                Expression::AssignmentExpression(assignment) => {
                    if assignment.operator != oxc_syntax::operator::AssignmentOperator::Assign {
                        self.error(
                            assignment.left.span().end,
                            "Only '=' operator can be used for specifying default value.",
                        );
                    }
                }
                Expression::ArrayExpression(array) => {
                    let count = array.elements.len();
                    for (i, element) in array.elements.iter().enumerate() {
                        match element {
                            ArrayExpressionElement::SpreadElement(spread) => {
                                self.spread(spread.span, &spread.argument, i + 1 == count);
                            }
                            ArrayExpressionElement::Elision(_) => {}
                            other => {
                                if let Some(inner) = other.as_expression() {
                                    self.target(inner);
                                }
                            }
                        }
                    }
                }
                Expression::ObjectExpression(object) => {
                    let count = object.properties.len();
                    for (i, property) in object.properties.iter().enumerate() {
                        match property {
                            ObjectPropertyKind::SpreadProperty(spread) => {
                                self.spread(spread.span, &spread.argument, i + 1 == count);
                            }
                            ObjectPropertyKind::ObjectProperty(property) => {
                                self.target(&property.value);
                            }
                        }
                    }
                }
                Expression::ParenthesizedExpression(inner) => {
                    self.error(inner.span.start, "Parenthesized pattern");
                }
                // acorn-typescript takes the expression under a cast.
                Expression::TSAsExpression(cast) if self.ts => self.target(&cast.expression),
                Expression::TSSatisfiesExpression(cast) if self.ts => {
                    self.target(&cast.expression);
                }
                Expression::TSTypeAssertion(cast) if self.ts => self.target(&cast.expression),
                Expression::TSNonNullExpression(cast) if self.ts => self.target(&cast.expression),
                other if self.ts && other.is_member_expression() => {
                    self.error(other.span().start, "Binding member expression");
                }
                other => self.error(other.span().start, "Assigning to rvalue"),
            }
        }
    }
    let mut check = Check {
        text: parameters,
        ts,
        first: None,
    };
    match &group.expression {
        Expression::SequenceExpression(sequence) => {
            for parameter in &sequence.expressions {
                check.target(parameter);
            }
        }
        single => check.target(single),
    }
    Some(
        check
            .first
            .map(|(at, message)| Candidate::at(open as u32 + at, message)),
    )
}

/// Whether `at` lies inside an async function.
fn in_async_function(program: &Program<'_>, at: u32) -> bool {
    struct Finder {
        at: u32,
        innermost: Option<bool>,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_function(&mut self, it: &oxc_ast::ast::Function<'a>, flags: ScopeFlags) {
            // A function's own name belongs to the code around it.
            let in_name = it
                .id
                .as_ref()
                .is_some_and(|id| id.span.start <= self.at && self.at < id.span.end);
            if it.span.start <= self.at && self.at < it.span.end && !in_name {
                self.innermost = Some(it.r#async);
                walk::walk_function(self, it, flags);
            }
        }

        fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
            if it.span.start <= self.at && self.at < it.span.end {
                self.innermost = Some(it.r#async);
                walk::walk_arrow_function_expression(self, it);
            }
        }
    }
    let mut finder = Finder {
        at,
        innermost: None,
    };
    finder.visit_program(program);
    finder.innermost == Some(true)
}

/// Whether the `=>` at `arrow` follows the parenthesized start of a type
/// alias's type (`type A = (…) => …`), where the parentheses are no
/// arrow function's parameters.
fn in_type_alias(text: &str, arrow: u32) -> bool {
    let before = &text[..arrow as usize];
    let statement = before
        .rfind(['\n', ';'])
        .map_or(before, |i| &before[i + 1..])
        .trim_start();
    statement.starts_with("type ")
        || statement.starts_with("export type ")
        || statement.starts_with("declare type ")
}

/// The text from the start of the class, interface or type literal
/// member list that `at` lies in back to the previous statement: what
/// kind of body it is.
fn enclosing_body_header(text: &str, at: u32) -> &str {
    let before = &text[..at as usize];
    let mut depth = 0usize;
    for (i, c) in before.char_indices().rev() {
        match c {
            '}' => depth += 1,
            '{' if depth == 0 => {
                let head = &before[..i];
                let from = head.rfind(['\n', ';', '}']).map_or(0, |j| j + 1);
                return head[from..].trim();
            }
            '{' => depth -= 1,
            _ => {}
        }
    }
    ""
}

/// Whether `at` is a member of a class body (not of an interface or a
/// type literal).
fn in_class_body(text: &str, at: u32) -> bool {
    let header = enclosing_body_header(text, at);
    header.split_whitespace().any(|word| word == "class")
        || header.starts_with("class")
        || header.contains(" class ")
}

/// Where the class member whose name is at `key` starts, modifiers
/// included.
fn class_member_start(program: &Program<'_>, key: u32) -> Option<u32> {
    struct Finder {
        key: u32,
        found: Option<u32>,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_class_element(&mut self, it: &oxc_ast::ast::ClassElement<'a>) {
            if it
                .property_key()
                .is_some_and(|k| k.span().start == self.key)
            {
                self.found = Some(it.span().start);
            }
            walk::walk_class_element(self, it);
        }
    }
    let mut finder = Finder { key, found: None };
    finder.visit_program(program);
    finder.found
}

/// Whether a class field's value starts at `value`.
fn class_field_value_at(program: &Program<'_>, value: u32) -> bool {
    struct Finder {
        value: u32,
        found: bool,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_property_definition(&mut self, it: &oxc_ast::ast::PropertyDefinition<'a>) {
            if it
                .value
                .as_ref()
                .is_some_and(|v| v.span().start == self.value)
            {
                self.found = true;
            }
            walk::walk_property_definition(self, it);
        }
    }
    let mut finder = Finder {
        value,
        found: false,
    };
    finder.visit_program(program);
    finder.found
}

/// Where the function whose body starts at `body` starts.
fn function_start_with_body_at(program: &Program<'_>, body: u32) -> Option<u32> {
    struct Finder {
        body: u32,
        found: Option<u32>,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_function(&mut self, it: &oxc_ast::ast::Function<'a>, flags: ScopeFlags) {
            if matches!(
                it.r#type,
                oxc_ast::ast::FunctionType::FunctionDeclaration
                    | oxc_ast::ast::FunctionType::TSDeclareFunction
            ) && it.body.as_ref().is_some_and(|b| b.span.start == self.body)
            {
                self.found = Some(it.span.start);
            }
            walk::walk_function(self, it, flags);
        }
    }
    let mut finder = Finder { body, found: None };
    finder.visit_program(program);
    finder.found
}

/// acorn-typescript's answer to an error oxc reports for TypeScript
/// syntax, one entry per error in acorn-typescript's own table that has
/// an oxc counterpart. `Some(None)`: acorn-typescript accepts it.
/// `None`: not decided here.
fn acorn_typescript_wording(
    diagnostic: &OxcDiagnostic,
    program: &Program<'_>,
    text: &str,
) -> Option<Option<Candidate>> {
    let message = diagnostic.message.as_ref();
    let span = diagnostic.labels.first().map(|l| l.span())?;
    let (s, e) = (span.start, span.end);
    let last = diagnostic.labels.last().map_or(s, |l| l.span().start);
    let at = |offset: u32, message: &str| Some(Some(Candidate::at(offset, message)));
    let unexpected = |offset: u32| Some(Some(Candidate::at(offset, "Unexpected token")));
    let byte = |offset: u32| text.as_bytes().get(offset as usize).copied();
    let slice = text.get(s as usize..e as usize).unwrap_or_default();
    let before = text[..s as usize].trim_end();
    match message {
        "Initializers are not allowed in ambient contexts." => {
            // On a class field, at its `=`.
            if class_field_value_at(program, s)
                && let Some(equals) = before.strip_suffix('=').map(str::len)
            {
                return at(equals as u32, message);
            }
            at(s, message)
        }
        "'override' modifier cannot appear on an index signature."
        | "A required element cannot follow an optional element."
        | "'readonly' type modifier is only permitted on array and tuple literal types."
        | "A parameter property may not be declared using a binding pattern."
        | "The 'type' modifier cannot be used on a named export when 'export type' is used on its export statement."
        | "The 'type' modifier cannot be used on a named import when 'import type' is used on its import statement." => {
            at(s, message)
        }
        "'readonly' modifier can only appear on a property declaration or index signature." => {
            if in_class_body(text, s) {
                at(s, "Class methods cannot have the 'readonly' modifier.")
            } else {
                at(s, message)
            }
        }
        "Type parameters cannot appear on a constructor declaration" => at(
            s,
            "Type parameters cannot appear on a constructor declaration.",
        ),
        "'abstract' modifier cannot appear on an index signature." => {
            at(s, "Index signatures cannot have the 'abstract' modifier.")
        }
        "'declare' modifier cannot appear on an index signature." => {
            at(s, "Index signatures cannot have the 'declare' modifier.")
        }
        "'public' modifier cannot appear on an index signature."
        | "'private' modifier cannot appear on an index signature."
        | "'protected' modifier cannot appear on an index signature." => at(
            s,
            &format!("Index signatures cannot have an accessibility modifier ('{slice}')."),
        ),
        "'abstract' modifier cannot be used with a private identifier." => {
            at(s, "Private elements cannot have the 'abstract' modifier.")
        }
        "An accessibility modifier cannot be used with a private identifier." => {
            // A method when a parameter list follows the name.
            let name = next_token_start(text, e);
            let after = next_token_start(text, identifier_end(text, name + 1));
            let kind = if matches!(byte(after), Some(b'(' | b'<')) {
                "methods"
            } else {
                "elements"
            };
            at(
                s,
                &format!("Private {kind} cannot have an accessibility modifier ('{slice}')."),
            )
        }
        "'declare' modifier cannot appear on class elements of this kind." => {
            at(s, "Class methods cannot have the 'declare' modifier.")
        }
        "'declare' modifier cannot be used here." if in_class_body(text, s) => {
            at(s, "Class methods cannot have the 'declare' modifier.")
        }
        "An implementation cannot be declared in ambient contexts." => {
            // Only for a function declaration: a declared class may give
            // its methods a body.
            match function_start_with_body_at(program, s) {
                Some(start) => at(start, message),
                None => Some(None),
            }
        }
        "'extends' list cannot be empty." | "'implements' list cannot be empty." => {
            at(next_token_start(text, e), message)
        }
        "Type argument list cannot be empty." => {
            // In a call acorn-typescript does not get as far as a type
            // argument list.
            if matches!(byte(next_token_start(text, e)), Some(b'(' | b'`')) {
                return unexpected(s + 1);
            }
            at(s + 1, message)
        }
        "Type parameter list cannot be empty." => at(next_token_start(text, e), message),
        "An import alias cannot use 'import type'" => {
            let equals = slice.find('=')? as u32;
            at(
                next_token_start(text, s + equals + 1),
                "An import alias can not use 'import type'.",
            )
        }
        "An instantiation expression cannot be followed by a property access." => {
            let close = slice.rfind('>')? as u32;
            at(
                s + close + 1,
                "Invalid property access after an instantiation expression. You can either wrap the instantiation expression in parentheses, or delete the type arguments.",
            )
        }
        "A type-only import can specify a default import or named bindings, but not both." => {
            let statement = text[..s as usize].rfind("import")?;
            at(statement as u32, message)
        }
        "String literal expected." if before.ends_with("import(") => {
            at(s, "Argument in a type import must be a string literal.")
        }
        "An arrow function cannot have a `this` parameter." => at(s, "Assigning to rvalue"),
        "A rest parameter cannot be optional" => {
            let rest = text[..s as usize].rfind("...")?;
            at(
                rest as u32,
                "A binding pattern parameter cannot be optional in an implementation signature.",
            )
        }
        "Modifiers cannot appear here." => {
            // A modifier before a static block, reported inside the block.
            let block = text[e as usize..].find('{')? as u32;
            at(
                e + block + 1,
                "Static class blocks cannot have any modifier.",
            )
        }
        "'extends' clause must precede 'implements' clause" => unexpected(last),
        "Classes can only extend a single class." => {
            unexpected(text[..s as usize].rfind(',')? as u32)
        }
        "Interface declaration cannot have 'implements' clause."
        | "An enum member cannot have a numeric name." => unexpected(s),
        "A property cannot be both optional and definite." => unexpected(s + 1),
        "Only string literals are allowed as module attribute values." => {
            at(e, "Only string is supported as an attribute value")
        }
        // acorn-typescript accepts these.
        "'new super()' is not allowed"
        | "Dynamic imports can only accept a module specifier and an optional set of attributes as arguments" => {
            Some(None)
        }
        "Tagged template expressions are not permitted in an optional chain"
            if s > 0 && byte(s - 1) == Some(b'>') =>
        {
            Some(None)
        }
        "Declaration cannot appear in a single-statement context" => {
            // A type-level declaration is one acorn-typescript reads
            // anywhere a statement goes.
            let word = slice.split_whitespace().next().unwrap_or_default();
            matches!(
                word,
                "type" | "interface" | "namespace" | "module" | "declare" | "enum" | "abstract"
            )
            .then_some(None)
        }
        _ => {
            if let Some(rest) = message
                .strip_prefix("Method '")
                .or_else(|| message.strip_prefix("Accessor '"))
            {
                if rest.ends_with("cannot have an implementation because it is marked abstract.") {
                    let name = rest.split('\'').next().unwrap_or_default();
                    return at(
                        class_member_start(program, s).unwrap_or(s),
                        &format!(
                            "Method '{name}' cannot have an implementation because it is marked abstract."
                        ),
                    );
                }
            }
            if message.starts_with("Property '")
                && message.ends_with("cannot have an initializer because it is marked abstract.")
            {
                return at(next_token_start(text, e), message);
            }
            if message.ends_with("' modifier cannot appear on a type member.") {
                // acorn-typescript passes its message template itself.
                return at(
                    next_token_start(text, e),
                    "({ modifier }) => `'${modifier}' modifier cannot appear on a type member.`",
                );
            }
            if message.ends_with(
                "' modifier can only appear on a type parameter of a class, interface or type alias.",
            ) {
                return at(
                    next_token_start(text, e),
                    "({ modifier }) => `'${modifier}' modifier can only appear on a type parameter of a class, interface or type alias.`",
                );
            }
            if message.ends_with("' modifier cannot be used on a type parameter.") {
                // Read as the parameter's name, so what follows it is
                // unexpected, unless the name itself is reserved.
                if RESERVED_IN_STRICT_MODE.contains(&slice) {
                    return at(s, &format!("The keyword '{slice}' is reserved"));
                }
                return unexpected(next_token_start(text, e));
            }
            if message.ends_with("' modifier cannot be used here.") {
                // The modifier is read as a name: a reserved one is
                // refused there, and after any other the next token is
                // unexpected. `abstract` before a word that is no
                // keyword (`let`, `enum`, `type`, `namespace`) starts a
                // declaration that is then refused at `abstract`.
                if RESERVED_IN_STRICT_MODE.contains(&slice) {
                    return at(s, &format!("The keyword '{slice}' is reserved"));
                }
                let next = next_token_start(text, e);
                if slice == "abstract" && !KEYWORDS.contains(&word_at(text, next)) {
                    return unexpected(s);
                }
                return unexpected(next);
            }
            if message == "Identifier expected. 'this' is a reserved word that cannot be used here."
            {
                // A `this` parameter is read wherever it stands.
                return Some(None);
            }
            if message == "Unexpected token" {
                // What follows `implements` / `export declare`.
                if before.ends_with("implements") {
                    return at(s, "'implements' list cannot be empty.");
                }
                if before.ends_with("export declare") {
                    return at(
                        s,
                        "'export declare' must be followed by an ambient declaration.",
                    );
                }
                // A decorator before something that is no class.
                if let Some(decorator) = before.rfind('@') {
                    let expression = &before[decorator + 1..];
                    let is_decorator = !expression.is_empty()
                        && !expression.contains(char::is_whitespace)
                        && !expression.contains([';', '{', '}']);
                    if is_decorator && !slice.starts_with("class") && !slice.starts_with("export") {
                        return at(
                            s,
                            "Leading decorators must be attached to a class declaration.",
                        );
                    }
                }
            }
            if message.starts_with("Expected `;` but found") {
                // The same modifier written twice before a member.
                let mut words = before.split_whitespace().rev();
                if let (Some(a), Some(b)) = (words.next(), words.next())
                    && a == b
                    && matches!(
                        a,
                        "static" | "declare" | "abstract" | "override" | "readonly"
                    )
                    && in_class_body(text, s)
                {
                    return at(s, &format!("Duplicate modifier: '{a}'."));
                }
            }
            if message.starts_with("Expected `,` or `]` but found `:`") && byte(last) == Some(b'[')
            {
                // A tuple member whose label is no plain name, at the
                // member.
                let member = text[..s as usize].rfind([',', '['])? as u32;
                return at(
                    next_token_start(text, member + 1),
                    "Tuple members must be labeled with a simple identifier.",
                );
            }
            if message.starts_with("Expected `,` or `)` but found `:`") && byte(last) == Some(b'(')
            {
                // After a parameter's default value. In parentheses that
                // are no parameter list acorn-typescript reads a type
                // annotation and goes on.
                let head = text[..last as usize].trim_end();
                let parameters = head
                    .ends_with(|c: char| oxc_syntax::identifier::is_identifier_part(c) || c == '>')
                    && !head.ends_with("return");
                if !parameters {
                    return Some(None);
                }
                return at(
                    s,
                    "Type annotations must come before default assignments, e.g. instead of `age = 25: number` use `age: number = 25`.",
                );
            }
            // Parentheses holding a type annotation and no arrow after
            // them: acorn-typescript reads a typed expression.
            if (message.starts_with("Expected `=>` but found")
                || message == "Line terminator not permitted before arrow")
                && !text[s as usize..].trim_start().starts_with("=>")
                && before.contains(':')
            {
                return Some(None);
            }
            None
        }
    }
}

/// acorn's own wording and position for an error oxc words differently,
/// one entry per `raise` site in acorn that has an oxc counterpart.
fn acorn_wording(
    diagnostic: &OxcDiagnostic,
    all: &[OxcDiagnostic],
    text: &str,
    mode: Mode,
) -> Option<Candidate> {
    let message = diagnostic.message.as_ref();
    let span = diagnostic.labels.first().map(|l| l.span())?;
    let (s, e) = (span.start, span.end);
    let last = diagnostic.labels.last().map_or(s, |l| l.span().start);
    let at = |offset: u32, message: &str| Some(Candidate::at(offset, message));
    let byte = |offset: u32| text.as_bytes().get(offset as usize).copied();
    let slice = text.get(s as usize..e as usize).unwrap_or_default();
    match message {
        "'import statement' declaration can only be used at the top level of a module"
        | "'export statement' declaration can only be used at the top level of a module" => {
            at(s, "'import' and 'export' may only appear at the top level")
        }
        "A 'default' clause cannot appear more than once in a 'switch' statement." => {
            at(last, "Multiple default clauses")
        }
        // At the end of the `throw` keyword.
        "Illegal newline after throw" => at(e, "Illegal newline after throw"),
        "Missing catch or finally clause" => {
            // At the `try` this block belongs to.
            let before = &text[..s as usize];
            let block = before.trim_end().strip_suffix('}')?;
            let mut depth = 1usize;
            let mut open = None;
            for (i, c) in block.char_indices().rev() {
                match c {
                    '}' => depth += 1,
                    '{' => {
                        depth -= 1;
                        if depth == 0 {
                            open = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let keyword = block[..open?].trim_end().strip_suffix("try")?.len();
            at(keyword as u32, "Missing catch or finally clause")
        }
        "Classes may not have a static property named 'prototype'"
            if byte(next_token_start(text, e)) == Some(b'(') =>
        {
            at(s, "Classes may not have a static property named prototype")
        }
        "A 'set' accessor cannot have rest parameter." => at(s, "Setter cannot use rest params"),
        "A string literal cannot be used as an exported binding without `from`" => at(
            s,
            "A string literal cannot be used as an exported binding without `from`.",
        ),
        "An export name cannot include a unicode lone surrogate" => {
            at(s, "An export name cannot include a lone surrogate.")
        }
        "A rest element cannot have an initializer." => {
            at(s, "Rest elements cannot have a default value")
        }
        "Invalid rest element target in destructuring assignment" if slice.contains('=') => {
            at(s, "Rest elements cannot have a default value")
        }
        "Only '=' operator can be used for specifying default value." => {
            // At the end of the target, before the operator.
            let equals = slice.find('=')?;
            let left = slice[..equals]
                .trim_end_matches(|c: char| !c.is_alphanumeric() && !"_$)]}".contains(c));
            at(s + left.len() as u32, message)
        }
        "Invalid optional chain from new expression." => at(
            s,
            "Optional chaining cannot appear in the callee of new expressions",
        ),
        // acorn-typescript accepts it.
        "'new super()' is not allowed" => {
            (!mode.ts).then(|| Candidate::at(s, "Invalid use of 'super'"))
        }
        "'super' can only be used with function calls or in property accesses" => {
            at(next_token_start(text, e), "Unexpected token")
        }
        "The left-hand side of a `for...of` statement may not start with `let`" => {
            at(s, "The keyword 'let' is reserved")
        }
        // acorn reads `async of` as the start of an arrow function.
        "The left-hand side of a `for...of` statement may not be `async`" => {
            let of = next_token_start(text, e);
            at(next_token_start(text, of + 2), "Unexpected token")
        }
        "Only a single declaration is allowed in a `for...of` statement" => {
            at(next_token_start(text, e), "Unexpected token")
        }
        "Unexpected private identifier" if text[..s as usize].contains("class") => at(
            s,
            "Private identifier can only be left side of binary expression",
        ),
        "A 'yield' expression is only allowed in a generator body." => {
            at(s, "The keyword 'yield' is reserved")
        }
        "A 'return' statement cannot be used inside a class static block." => {
            at(s, "'return' outside of function")
        }
        "A module cannot have multiple default exports." => {
            // acorn-typescript does not track exports.
            (!mode.ts).then(|| Candidate::at(last, "Duplicate export 'default'"))
        }
        "Keywords cannot contain escape characters" => {
            let word = unescaped(slice);
            // The property of `import.meta` / `new.target`.
            let before = text[..s as usize].trim_end();
            if let Some(object) = before.strip_suffix('.').map(str::trim_end) {
                for (keyword, name) in [("import", "import.meta"), ("new", "new.target")] {
                    if let Some(start) = object.strip_suffix(keyword).map(str::len) {
                        return at(
                            start as u32,
                            &format!("'{name}' must not contain escaped characters"),
                        );
                    }
                }
            }
            at(s, &format!("Escape sequence in keyword {word}"))
        }
        "The only valid property accesses on import are `import.meta`, `import.source()`, and `import.defer()`" =>
        {
            let dot = slice.find('.')? as u32;
            at(
                next_token_start(text, s + dot + 1),
                "The only valid meta property for import is 'import.meta'",
            )
        }
        "The only valid meta property for new is new.target" => {
            let dot = slice.find('.')? as u32;
            at(
                next_token_start(text, s + dot + 1),
                "The only valid meta property for new is 'new.target'",
            )
        }
        "Dynamic imports can only accept a module specifier and an optional set of attributes as arguments" => {
            at(next_token_start(text, e), "Unexpected token")
        }
        "import() requires a specifier." => at(e.saturating_sub(1), "Unexpected token"),
        "Argument of dynamic import cannot be a spread element." => at(s, "Unexpected token"),
        "Invalid Unicode escape sequence"
            if s > 0 && byte(s - 1) == Some(b'\\') && byte(s) != Some(b'u') =>
        {
            at(s, "Expecting Unicode escape sequence \\uXXXX")
        }
        _ => {
            if let Some(flag) = message
                .strip_prefix("Unexpected flag ")
                .filter(|rest| rest.ends_with("in regular expression literal"))
            {
                let _ = flag;
                // The regular expression's start comes with a later
                // report: the one whose span starts at the `/` and
                // holds this flag.
                let start = all
                    .iter()
                    .filter_map(|d| d.labels.first().map(|l| l.span()))
                    .find(|span| span.start < s && span.end >= e && byte(span.start) == Some(b'/'))?
                    .start;
                return Some(
                    Candidate::at(start + 1, "Invalid regular expression flag").in_token(start),
                );
            }
            if let Some(rest) = message.strip_prefix("Identifier `") {
                // A repeated key in an import's `with { … }`.
                let name = rest.split('`').next().unwrap_or_default();
                let before = &text[..last as usize];
                let attributes = before.rfind("with")?;
                let inside = &before[attributes + 4..];
                if rest.ends_with("has already been declared")
                    && inside.trim_start().starts_with('{')
                    && !inside.contains('}')
                {
                    if mode.ts {
                        // acorn-typescript words it differently, once it
                        // has read the attribute's value.
                        let colon = text[last as usize..].find(':')? as u32;
                        let value = next_token_start(text, last + colon + 1);
                        let quote = byte(value).filter(|q| matches!(q, b'"' | b'\''))?;
                        let close = text[value as usize + 1..].find(char::from(quote))? as u32;
                        return at(value + 1 + close + 1, "Duplicated key in attributes");
                    }
                    return at(last, &format!("Duplicate attribute key '{name}'"));
                }
            }
            number_error(message, text, s)
        }
    }
}

/// acorn's error for a malformed numeric literal, which oxc reports at
/// the character that does not fit (`at`).
fn number_error(message: &str, text: &str, at: u32) -> Option<Candidate> {
    let unfit = message == "Unexpected end of file"
        || message.starts_with("Invalid Character `")
        || message == "Invalid characters after number";
    if !unfit {
        return None;
    }
    // The sign of an exponent is part of the literal.
    let before = text.get(..at as usize)?;
    let unsigned = before.trim_end_matches(['+', '-']);
    let end = if unsigned.ends_with(['e', 'E']) {
        unsigned.len() as u32
    } else {
        at
    };
    let start = number_start(text, end);
    if !text.as_bytes().get(start as usize)?.is_ascii_digit() {
        return None;
    }
    let (at, message) = acorn_number_error(text, start as usize)?;
    Some(Candidate::at(at, message).in_token(start))
}

/// The error acorn's tokenizer raises reading the number that starts at
/// `start` (a digit), and where: its `readNumber`, `readRadixNumber` and
/// `readInt`, in strict mode. `None` when it reads the number whole.
fn acorn_number_error(text: &str, start: usize) -> Option<(u32, &'static str)> {
    let bytes = text.as_bytes();
    let byte = |i: usize| bytes.get(i).copied();
    let identifier_start = |i: usize| {
        text.get(i..)
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c == '$' || c == '_' || oxc_syntax::identifier::is_identifier_start(c))
    };
    // `readInt`: where the digits end and whether there were any, or
    // the error a numeric separator is.
    let read_int = |from: usize, radix: u32, plain_integer: bool| {
        let legacy_octal = plain_integer && byte(from) == Some(b'0');
        let mut pos = from;
        let mut last = 0u8;
        while let Some(code) = byte(pos) {
            if code == b'_' {
                if legacy_octal {
                    return Err((
                        pos as u32,
                        "Numeric separator is not allowed in legacy octal numeric literals",
                    ));
                }
                if last == b'_' {
                    return Err((
                        pos as u32,
                        "Numeric separator must be exactly one underscore",
                    ));
                }
                if pos == from {
                    return Err((
                        pos as u32,
                        "Numeric separator is not allowed at the first of digits",
                    ));
                }
                last = code;
                pos += 1;
                continue;
            }
            if !char::from(code).is_digit(radix) {
                break;
            }
            last = code;
            pos += 1;
        }
        if last == b'_' {
            return Err((
                pos as u32 - 1,
                "Numeric separator is not allowed at the last of digits",
            ));
        }
        Ok((pos, pos != from))
    };
    let after_number = |pos: usize| {
        identifier_start(pos).then_some((pos as u32, "Identifier directly after number"))
    };
    let replay = || -> Result<Option<(u32, &'static str)>, (u32, &'static str)> {
        let radix = match (byte(start), byte(start + 1).map(|b| b.to_ascii_lowercase())) {
            (Some(b'0'), Some(b'x')) => Some((16, "Expected number in radix 16")),
            (Some(b'0'), Some(b'o')) => Some((8, "Expected number in radix 8")),
            (Some(b'0'), Some(b'b')) => Some((2, "Expected number in radix 2")),
            _ => None,
        };
        if let Some((radix, expected)) = radix {
            let (pos, any) = read_int(start + 2, radix, false)?;
            if !any {
                return Ok(Some((start as u32 + 2, expected)));
            }
            if byte(pos) == Some(b'n') {
                return Ok(None);
            }
            return Ok(after_number(pos));
        }
        let invalid = Some((start as u32, "Invalid number"));
        let (mut pos, any) = read_int(start, 10, true)?;
        if !any {
            return Ok(invalid);
        }
        // More than one digit after a leading zero: a legacy octal
        // literal, which strict code has none of.
        if pos - start >= 2 && byte(start) == Some(b'0') {
            return Ok(invalid);
        }
        if byte(pos) == Some(b'n') {
            return Ok(after_number(pos + 1));
        }
        if byte(pos) == Some(b'.') {
            (pos, _) = read_int(pos + 1, 10, false)?;
        }
        if matches!(byte(pos), Some(b'e' | b'E')) {
            pos += 1;
            if matches!(byte(pos), Some(b'+' | b'-')) {
                pos += 1;
            }
            let (end, any) = read_int(pos, 10, false)?;
            if !any {
                return Ok(invalid);
            }
            pos = end;
        }
        Ok(after_number(pos))
    };
    replay().unwrap_or_else(Some)
}

/// `word` with its `\uXXXX` / `\u{…}` escapes decoded.
fn unescaped(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(escape) = rest.find("\\u") {
        out.push_str(&rest[..escape]);
        let after = &rest[escape + 2..];
        let (digits, tail) = match after.strip_prefix('{') {
            Some(braced) => match braced.find('}') {
                Some(close) => (&braced[..close], &braced[close + 1..]),
                None => ("", after),
            },
            None => after.split_at(after.len().min(4)),
        };
        match u32::from_str_radix(digits, 16)
            .ok()
            .and_then(char::from_u32)
        {
            Some(c) => out.push(c),
            None => out.push_str("\\u"),
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Words strict-mode code may not use as identifiers.
pub(crate) const RESERVED_IN_STRICT_MODE: &[&str] = &[
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
    fn escaped_operator_after_a_field_initializer() {
        let first = |body: &str| as_written(&format!("<script>\nclass C {{ {body} }}\n</script>"));
        let error = |at: &str, word: &str| Some(format!("{at} Escape sequence in keyword {word}"));
        assert_eq!(first("x = a\n\\u0069n = 1"), error("2:0", "in"));
        assert_eq!(
            first("x = a\n\\u0069nstanceof = 1"),
            error("2:0", "instanceof")
        );
        assert_eq!(first("x = a\n\\u0069n() {}"), error("2:0", "in"));
        assert_eq!(first("x = () => a\n\\u0069n = 1"), error("2:0", "in"));
        assert_eq!(first("x = (() => {})\n\\u0069n = 1"), error("2:0", "in"));
        assert_eq!(first("x = a // c\n\\u0069n = 1"), error("2:0", "in"));
        for body in [
            "x = a;\n\\u0069n = 1",
            "x\n\\u0069n = 1",
            "x = () => {}\n\\u0069n = 1",
            "x = a ? b : () => {}\n\\u0069n = 1",
            "x = y = () => {}\n\\u0069n = 1",
            "x = a\nstatic \\u0069n = 1",
            "x = a; \\u0069n = 1",
        ] {
            assert_eq!(first(body), None, "{body}");
        }
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
            as_written("<script lang=\"ts\">\nclass K {\n  readonly accessor x = 1\n}\n</script>")
                .as_deref(),
            Some("0:11 'accessor' modifier cannot be used with 'readonly' modifier.")
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
