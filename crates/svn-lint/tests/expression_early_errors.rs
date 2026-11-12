//! Errors acorn raises for an expression that parses.
//!
//! The compiler has acorn read each template expression as strict-mode
//! module code. acorn rejects more than a syntax error there: a private
//! name outside a class that declares it (`#x in y`, `a.#x`), a reserved
//! word used as a name (`yield`, `let`, `static`), assigning to `eval`,
//! a parameter named twice, an octal literal, `delete` of a variable,
//! `??` mixed with `||`. Each case below is what svelte 5.57.2's
//! `parse()` reports for the template.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32)>;

/// `(typescript, template, the compiler's error: code, message, offset)`.
/// Offsets count from the start of the file, script line included.
const CASES: &[(bool, &str, Expected)] = &[
    (
        false,
        "{#if #x in y}a{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            23,
        )),
    ),
    (
        true,
        "{#if #x in y}a{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        false,
        "{#if a}b{:else if #x in y}c{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        true,
        "{#if a}b{:else if #x in y}c{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            46,
        )),
    ),
    (
        false,
        "{@html #x in y}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            25,
        )),
    ),
    (
        true,
        "{@html #x in y}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        false,
        "{#key #x in y}a{/key}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            24,
        )),
    ),
    (
        true,
        "{#key #x in y}a{/key}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{#each #x in y as z}a{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            25,
        )),
    ),
    (
        true,
        "{#each #x in y as z}a{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        false,
        "{#await #x in y}a{/await}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            26,
        )),
    ),
    (
        true,
        "{#await #x in y}a{/await}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        false,
        "{#if x}{@const a = #x in y}{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (
        true,
        "{#if x}{@const a = #x in y}{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            47,
        )),
    ),
    (
        false,
        "{#if #x}a{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            23,
        )),
    ),
    (
        true,
        "{#if #x}a{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        false,
        "{a.#x}",
        Some((
            "js_parse_error",
            "Private field '#x' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            21,
        )),
    ),
    (
        true,
        "{a.#x}",
        Some((
            "js_parse_error",
            "Private field '#x' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            31,
        )),
    ),
    (
        false,
        "{a?.#x}",
        Some((
            "js_parse_error",
            "Private field '#x' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            22,
        )),
    ),
    (
        true,
        "{a?.#x}",
        Some((
            "js_parse_error",
            "Private field '#x' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        false,
        "{a && #x in y}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            24,
        )),
    ),
    (
        true,
        "{a && #x in y}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{f(#x in y)}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            21,
        )),
    ),
    (
        true,
        "{f(#x in y)}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            31,
        )),
    ),
    (false, "{class { #x; m(o) { return #x in o } }}", None),
    (true, "{class { #x; m(o) { return #x in o } }}", None),
    (
        false,
        "{class { m(o) { return #y in o } }}",
        Some((
            "js_parse_error",
            "Private field '#y' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            41,
        )),
    ),
    (
        true,
        "{class { m(o) { return #y in o } }}",
        Some((
            "js_parse_error",
            "Private field '#y' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            51,
        )),
    ),
    (
        false,
        "{class { m(o) { return o.#y } }}",
        Some((
            "js_parse_error",
            "Private field '#y' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            43,
        )),
    ),
    (
        true,
        "{class { m(o) { return o.#y } }}",
        Some((
            "js_parse_error",
            "Private field '#y' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            53,
        )),
    ),
    (false, "{class { #x; m(o) { return o.#x } }}", None),
    (true, "{class { #x; m(o) { return o.#x } }}", None),
    (
        false,
        "{#each a as b (#x in b)}c{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        true,
        "{#each a as b (#x in b)}c{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            43,
        )),
    ),
    (
        false,
        "<p a={#x in y}></p>",
        Some((
            "block_invalid_placement",
            "{#x ...} block cannot be in attribute value\nhttps://svelte.dev/e/block_invalid_placement",
            23,
        )),
    ),
    (
        true,
        "<p a={#x in y}></p>",
        Some((
            "block_invalid_placement",
            "{#x ...} block cannot be in attribute value\nhttps://svelte.dev/e/block_invalid_placement",
            33,
        )),
    ),
    (
        false,
        "<p a={y.#x}></p>",
        Some((
            "js_parse_error",
            "Private field '#x' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            26,
        )),
    ),
    (
        true,
        "<p a={y.#x}></p>",
        Some((
            "js_parse_error",
            "Private field '#x' must be declared in an enclosing class\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        false,
        "<p {...#x in y}></p>",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            25,
        )),
    ),
    (
        true,
        "<p {...#x in y}></p>",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        false,
        "<p class:a={#x in y}></p>",
        Some((
            "block_invalid_placement",
            "{#x ...} block cannot be in attribute value\nhttps://svelte.dev/e/block_invalid_placement",
            29,
        )),
    ),
    (
        true,
        "<p class:a={#x in y}></p>",
        Some((
            "block_invalid_placement",
            "{#x ...} block cannot be in attribute value\nhttps://svelte.dev/e/block_invalid_placement",
            39,
        )),
    ),
    (
        false,
        "{@render f(#x in y)}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        true,
        "{@render f(#x in y)}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            39,
        )),
    ),
    (
        false,
        "{@debug #x}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            26,
        )),
    ),
    (
        true,
        "{@debug #x}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        false,
        "{yield}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{yield}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{static}",
        Some((
            "js_parse_error",
            "The keyword 'static' is reserved\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{static}",
        Some((
            "js_parse_error",
            "The keyword 'static' is reserved\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{a + yield}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            23,
        )),
    ),
    (
        true,
        "{a + yield}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (false, "{eval}", None),
    (true, "{eval}", None),
    (
        false,
        "{eval = 1}",
        Some((
            "js_parse_error",
            "Assigning to eval in strict mode\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{eval = 1}",
        Some((
            "js_parse_error",
            "Assigning to eval in strict mode\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{await}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            24,
        )),
    ),
    (
        true,
        "{await}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{#if yield}a{/if}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            23,
        )),
    ),
    (
        true,
        "{#if yield}a{/if}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        false,
        "{x => { let let = 1 }}",
        Some((
            "js_parse_error",
            "The keyword 'let' is reserved\nhttps://svelte.dev/e/js_parse_error",
            30,
        )),
    ),
    (
        true,
        "{x => { let let = 1 }}",
        Some((
            "js_parse_error",
            "The keyword 'let' is reserved\nhttps://svelte.dev/e/js_parse_error",
            40,
        )),
    ),
    (
        false,
        "{function(a, a) {}}",
        Some((
            "js_parse_error",
            "Argument name clash\nhttps://svelte.dev/e/js_parse_error",
            31,
        )),
    ),
    (
        true,
        "{function(a, a) {}}",
        Some((
            "js_parse_error",
            "Argument name clash\nhttps://svelte.dev/e/js_parse_error",
            41,
        )),
    ),
    (
        false,
        "{(a, a) => 1}",
        Some((
            "js_parse_error",
            "Argument name clash\nhttps://svelte.dev/e/js_parse_error",
            23,
        )),
    ),
    (
        true,
        "{(a, a) => 1}",
        Some((
            "js_parse_error",
            "Argument name clash\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        false,
        "{010}",
        Some((
            "js_parse_error",
            "Invalid number\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{010}",
        Some((
            "js_parse_error",
            "Invalid number\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{'\\01'}",
        Some((
            "js_parse_error",
            "Octal literal in strict mode\nhttps://svelte.dev/e/js_parse_error",
            20,
        )),
    ),
    (
        true,
        "{'\\01'}",
        Some((
            "js_parse_error",
            "Octal literal in strict mode\nhttps://svelte.dev/e/js_parse_error",
            30,
        )),
    ),
    (
        false,
        "{delete x}",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{delete x}",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{() => { with (a) {} }}",
        Some((
            "js_parse_error",
            "'with' in strict mode\nhttps://svelte.dev/e/js_parse_error",
            27,
        )),
    ),
    (
        true,
        "{() => { with (a) {} }}",
        Some((
            "js_parse_error",
            "'with' in strict mode\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (false, "{a.yield}", None),
    (true, "{a.yield}", None),
    (
        false,
        "{{ yield }}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            21,
        )),
    ),
    (
        true,
        "{{ yield }}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            31,
        )),
    ),
    (false, "{{ yield: 1 }}", None),
    (true, "{{ yield: 1 }}", None),
    (
        false,
        "{implements + 1}",
        Some((
            "js_parse_error",
            "The keyword 'implements' is reserved\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{implements + 1}",
        Some((
            "js_parse_error",
            "The keyword 'implements' is reserved\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "<p a={yield}></p>",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            24,
        )),
    ),
    (
        true,
        "<p a={yield}></p>",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{#each yield as a}b{/each}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            25,
        )),
    ),
    (
        true,
        "{#each yield as a}b{/each}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        false,
        "{#await yield}b{/await}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            26,
        )),
    ),
    (
        true,
        "{#await yield}b{/await}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        false,
        "{new.target}",
        Some((
            "js_parse_error",
            "'new.target' can only be used in functions and class static block\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{new.target}",
        Some((
            "js_parse_error",
            "'new.target' can only be used in functions and class static block\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (false, "{() => { return }}", None),
    (true, "{() => { return }}", None),
    (
        false,
        "{super.x}",
        Some((
            "js_parse_error",
            "'super' keyword outside a method\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{super.x}",
        Some((
            "js_parse_error",
            "'super' keyword outside a method\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (false, "{import.meta}", None),
    (true, "{import.meta}", None),
    (
        false,
        "{a ?? b || c}",
        Some((
            "js_parse_error",
            "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses\nhttps://svelte.dev/e/js_parse_error",
            26,
        )),
    ),
    (
        true,
        "{a ?? b || c}",
        Some((
            "js_parse_error",
            "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        false,
        "{-a ** b}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            22,
        )),
    ),
    (
        true,
        "{-a ** b}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (false, "{async () => await x}", None),
    (true, "{async () => await x}", None),
    (
        false,
        "{() => await x}",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function\nhttps://svelte.dev/e/js_parse_error",
            25,
        )),
    ),
    (
        true,
        "{() => await x}",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
];

fn source(ts: bool, template: &str) -> String {
    let script = if ts {
        "<script lang=\"ts\"></script>\n"
    } else {
        "<script></script>\n"
    };
    format!("{script}{template}")
}

#[test]
fn lint_reports_the_compiler_error() {
    let mut failures = Vec::new();
    for &(ts, template, expected) in CASES {
        let src = source(ts, template);
        let warnings = svn_lint::lint_file(
            &src,
            Path::new("t.svelte"),
            Some(true),
            CompatFeatures::MODERN,
        );
        let got = warnings
            .iter()
            .find(|w| w.is_error)
            .map(|w| (w.code.as_str(), w.message.as_str(), w.range.start));
        if got != expected {
            failures.push(format!(
                "ts={ts} {template:?}\n  got:  {got:?}\n  want: {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The check skips type-checking a component the compiler rejects, so
/// each rejected template must be detected as one.
#[test]
fn rejected_templates_are_detected() {
    let mut failures = Vec::new();
    for &(ts, template, expected) in CASES {
        let src = source(ts, template);
        let (doc, _) = svn_parser::parse_sections(&src);
        let (fragment, errors) = svn_parser::parse_all_template_runs(&src, &doc.template.text_runs);
        let rejected = errors.iter().any(|e| e.compiler_rejects())
            || svn_lint::template_parse_rejected(&fragment, &src, doc.script_lang());
        if rejected != expected.is_some() {
            failures.push(format!("ts={ts} {template:?}: rejected={rejected}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
