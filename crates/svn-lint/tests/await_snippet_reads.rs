//! How the compiler reads the value of a `then` / `catch` and the head of
//! a `{#snippet}`, which decides where those tags end.
//!
//! A `then` / `catch` value is read with the compiler's pattern reader:
//! its bracket matcher, then acorn, then the type annotation, which the
//! compiler reads after rewriting `?:` to `:` while keeping the rewritten
//! positions. So `v: { a?: number }` ends one character early, on the
//! type's own `}`, and that `}` closes the tag. A snippet's parameter list
//! is found by counting parentheses one character at a time, strings and
//! comments included, and then read by acorn as an arrow function's. Each
//! case below is what svelte 5.57.2's `parse()` reports for the template.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32)>;

/// `(typescript, template, the compiler's error: code, message, offset)`.
/// Offsets count from the start of the file, script line included.
const CASES: &[(bool, &str, Expected)] = &[
    (
        false,
        "{#await p then v: { a?: number }}x{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            31,
        )),
    ),
    (
        false,
        "{#await p}{:then v: { a?: number }}x{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            33,
        )),
    ),
    (
        false,
        "{#await p}{:catch e: { a?: number }}x{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            34,
        )),
    ),
    (
        false,
        "{#await p catch e: { a?: number }}x{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            32,
        )),
    ),
    (
        false,
        "{#await p}{:then {a]}x{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            37,
        )),
    ),
    (
        false,
        "{#await p then {a]}x{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            35,
        )),
    ),
    (
        false,
        "{#await p}{:catch [a}x{/await}",
        Some((
            "expected_token",
            "Expected token ]\nhttps://svelte.dev/e/expected_token",
            38,
        )),
    ),
    (false, "{#await p}{:then v}x{/await}", None),
    (false, "{#await p then {a, b}}x{/await}", None),
    (
        false,
        "{#snippet foo(a = \")\")}x{/snippet}",
        Some((
            "js_parse_error",
            "Unterminated string constant\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        false,
        "{#snippet foo(a = \"(\")}x{/snippet}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            52,
        )),
    ),
    (
        false,
        "{#snippet foo(/* ) */ a)}x{/snippet}",
        Some((
            "js_parse_error",
            "Unterminated comment\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (false, "{#snippet foo(a = () => 1)}x{/snippet}", None),
    (
        false,
        "{#snippet foo(a = `)`)}x{/snippet}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (false, "{#snippet foo(a)  }x{/snippet}", None),
    (false, "{#snippet foo(a, b = [1, 2])}x{/snippet}", None),
    (true, "{#await p then v: { a?: number }}x{/await}", None),
    (true, "{#await p}{:then v: { a?: number }}x{/await}", None),
    (true, "{#await p}{:catch e: { a?: number }}x{/await}", None),
    (true, "{#await p catch e: { a?: number }}x{/await}", None),
    (
        true,
        "{#await p}{:then {a]}x{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        true,
        "{#await p then {a]}x{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        true,
        "{#await p}{:catch [a}x{/await}",
        Some((
            "expected_token",
            "Expected token ]\nhttps://svelte.dev/e/expected_token",
            48,
        )),
    ),
    (true, "{#await p}{:then v}x{/await}", None),
    (true, "{#await p then {a, b}}x{/await}", None),
    (
        true,
        "{#snippet foo(a = \")\")}x{/snippet}",
        Some((
            "js_parse_error",
            "Unterminated string constant\nhttps://svelte.dev/e/js_parse_error",
            46,
        )),
    ),
    (
        true,
        "{#snippet foo(a = \"(\")}x{/snippet}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            62,
        )),
    ),
    (
        true,
        "{#snippet foo(/* ) */ a)}x{/snippet}",
        Some((
            "js_parse_error",
            "Unterminated comment\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (true, "{#snippet foo(a = () => 1)}x{/snippet}", None),
    (
        true,
        "{#snippet foo(a = `)`)}x{/snippet}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            47,
        )),
    ),
    (true, "{#snippet foo(a)  }x{/snippet}", None),
    (true, "{#snippet foo(a, b = [1, 2])}x{/snippet}", None),
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
        let got = warnings.iter().find(|w| w.is_error).map(|w| {
            (
                w.code.as_str(),
                w.message.as_str(),
                w.range.start,
                w.range.end,
            )
        });
        let want = expected.map(|(code, message, at)| (code, message, at, at));
        if got != want {
            failures.push(format!(
                "ts={ts} {template:?}\n  got:  {got:?}\n  want: {want:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A component whose template the compiler rejects leaves the run, and
/// one it accepts stays in it.
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
