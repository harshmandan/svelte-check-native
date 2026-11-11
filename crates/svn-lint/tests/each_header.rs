//! The compiler reads an `{#each}` header in its own order. It hands
//! acorn (acorn-typescript in a TypeScript component) the rest of the
//! template, which reads the longest expression it can. When acorn
//! throws, the compiler cuts the template at the last `as` before the
//! error and reads again, and only when no `as` is left does the error
//! stand. Without an `as` after the expression, a TypeScript `as` cast
//! that ends it is taken back as the context. Then come `as pattern`
//! (read with its own reader and checked by acorn), `, index`,
//! `(key)` and the closing `}`. In TypeScript, acorn reads `as` as a
//! cast, so nearly every header goes through those steps. Each case
//! below is what svelte 5.57.2's `parse()` reports for the template.
//!
//! Not covered: a pattern whose brackets do not pair up
//! (`{#each [] as {a]}`). Our parser finds the end of a tag by counting
//! braces, so it builds no `{#each}` there and reports its own error
//! instead; the component is rejected either way.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32)>;

/// `(typescript, template, the compiler's error: code, message, offset)`.
/// Offsets count from the start of the file, script line included.
const CASES: &[(bool, &str, Expected)] = &[
    (
        false,
        "{#each [] as {a b}}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{#each [] as {a b}: T}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{#each [] as q q}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            33,
        )),
    ),
    (
        false,
        "{#each [] as 1}{/each}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            31,
        )),
    ),
    (
        false,
        "{#each [] as q, 1}{/each}",
        Some((
            "expected_identifier",
            "Expected an identifier\nhttps://svelte.dev/e/expected_identifier",
            34,
        )),
    ),
    (
        false,
        "{#each [] as q, i i}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            36,
        )),
    ),
    (
        false,
        "{#each [] as q (q}{/each}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            35,
        )),
    ),
    (
        false,
        "{#each [] as q (q) x}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            37,
        )),
    ),
    (
        false,
        "{#each [] as q: }{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            29,
        )),
    ),
    (
        false,
        "{#each [] as q: string string}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            29,
        )),
    ),
    (
        false,
        "{#each [] as [a,}{/each}",
        Some((
            "expected_token",
            "Expected token ]\nhttps://svelte.dev/e/expected_token",
            34,
        )),
    ),
    (
        false,
        "{#each [] as q,}{/each}",
        Some((
            "expected_identifier",
            "Expected an identifier\nhttps://svelte.dev/e/expected_identifier",
            33,
        )),
    ),
    (
        false,
        "{#each [] as}{/each}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            30,
        )),
    ),
    (
        false,
        "{#each [] as q: /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            29,
        )),
    ),
    (
        false,
        "{#each [] as {a = }}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        false,
        "{#each [] as q = 1}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            33,
        )),
    ),
    (
        false,
        "{#each [] q}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            28,
        )),
    ),
    (
        false,
        "{#each [] as q, i, j}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            35,
        )),
    ),
    (
        false,
        "{#each [] as q i}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            33,
        )),
    ),
    (
        false,
        "{#each [] as q: T<}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            29,
        )),
    ),
    (
        false,
        "{#each a b as q}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            27,
        )),
    ),
    (
        false,
        "{#each [] as q (q) (q)}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            37,
        )),
    ),
    (
        false,
        "{#each [] as ...q}{/each}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            31,
        )),
    ),
    (
        false,
        "{#each [] as q as r}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            33,
        )),
    ),
    (
        false,
        "{#each [] as q: string = 1}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            29,
        )),
    ),
    (
        false,
        "{#each [], }{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{#each [] as {a: }}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        false,
        "{#each [] as q, i (}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (
        false,
        "{#each [] as q.a}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            32,
        )),
    ),
    (
        false,
        "{#each [] as q()}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        false,
        "{#each f(a b) as q}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{#each [] as [a}]}{/each}",
        Some((
            "expected_token",
            "Expected token ]\nhttps://svelte.dev/e/expected_token",
            33,
        )),
    ),
    (
        false,
        "{#each [] as q, i (q +)}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            40,
        )),
    ),
    (
        false,
        "{#each [] as q (q, )}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (
        false,
        "{#each foo. as x}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            33,
        )),
    ),
    (false, "{#each items as item}{/each}", None),
    (false, "{#each items as item, i (item.id)}{/each}", None),
    (
        false,
        "{#each items as unknown as number[] as item}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            42,
        )),
    ),
    (false, "{#each a, i}{/each}", None),
    (false, "{#each a, i (i)}{/each}", None),
    (false, "{#each items as { y = z }}{/each}", None),
    (false, "{#each Array.from({ length: 10 }), i}{/each}", None),
    (false, "{#each [] as {a = 1}, i (a)}{/each}", None),
    (
        false,
        "{#each [] as [a, b]: [number, string]}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            35,
        )),
    ),
    (
        false,
        "{#each [] as item: string, i (item)}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            32,
        )),
    ),
    (
        false,
        "{#each f(x as any) as q}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{#each (a as any) as q}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            28,
        )),
    ),
    (false, "{#each a as b, c}{/each}", None),
    (false, "{#each c ? x : y as T}{/each}", None),
    (false, "{#each a, b as item}{/each}", None),
    (false, "{#each [] as q (q.id)}{/each}", None),
    (false, "{#each [] as {a: {b}}}{/each}", None),
    (
        false,
        "{#each [] as q: Map<string, number>}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            29,
        )),
    ),
    (
        false,
        "{#each [] as q: (a: string) => void, i}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            29,
        )),
    ),
    (
        true,
        "{#each [] as {a b}}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            44,
        )),
    ),
    (
        true,
        "{#each [] as {a b}: T}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            44,
        )),
    ),
    (
        true,
        "{#each [] as q q}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            43,
        )),
    ),
    (
        true,
        "{#each [] as 1}{/each}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            41,
        )),
    ),
    (
        true,
        "{#each [] as q, 1}{/each}",
        Some((
            "expected_identifier",
            "Expected an identifier\nhttps://svelte.dev/e/expected_identifier",
            44,
        )),
    ),
    (
        true,
        "{#each [] as q, i i}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            46,
        )),
    ),
    (
        true,
        "{#each [] as q (q}{/each}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        true,
        "{#each [] as q (q) x}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        true,
        "{#each [] as q: }{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            44,
        )),
    ),
    (
        true,
        "{#each [] as q: string string}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            51,
        )),
    ),
    (
        true,
        "{#each [] as [a,}{/each}",
        Some((
            "expected_token",
            "Expected token ]\nhttps://svelte.dev/e/expected_token",
            44,
        )),
    ),
    (
        true,
        "{#each [] as q,}{/each}",
        Some((
            "expected_identifier",
            "Expected an identifier\nhttps://svelte.dev/e/expected_identifier",
            43,
        )),
    ),
    (
        true,
        "{#each [] as}{/each}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            40,
        )),
    ),
    (
        true,
        "{#each [] as q: /*c*/}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            49,
        )),
    ),
    (
        true,
        "{#each [] as {a = }}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            46,
        )),
    ),
    (
        true,
        "{#each [] as q = 1}{/each}",
        Some((
            "js_parse_error",
            "Assigning to rvalue\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        true,
        "{#each [] q}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            38,
        )),
    ),
    (
        true,
        "{#each [] as q, i, j}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        true,
        "{#each [] as q i}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            43,
        )),
    ),
    (
        true,
        "{#each [] as q: T<}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            46,
        )),
    ),
    (
        true,
        "{#each a b as q}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            37,
        )),
    ),
    (
        true,
        "{#each [] as q (q) (q)}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        true,
        "{#each [] as ...q}{/each}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            41,
        )),
    ),
    (true, "{#each [] as q as r}{/each}", None),
    (
        true,
        "{#each [] as q: string = 1}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            51,
        )),
    ),
    (
        true,
        "{#each [], }{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            39,
        )),
    ),
    (
        true,
        "{#each [] as {a: }}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            45,
        )),
    ),
    (
        true,
        "{#each [] as q, i (}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            47,
        )),
    ),
    (
        true,
        "{#each [] as q.a}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            42,
        )),
    ),
    (
        true,
        "{#each [] as q()}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            43,
        )),
    ),
    (
        true,
        "{#each f(a b) as q}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            39,
        )),
    ),
    (
        true,
        "{#each [] as [a}]}{/each}",
        Some((
            "expected_token",
            "Expected token ]\nhttps://svelte.dev/e/expected_token",
            43,
        )),
    ),
    (
        true,
        "{#each [] as q, i (q +)}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            50,
        )),
    ),
    (
        true,
        "{#each [] as q (q, )}{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            47,
        )),
    ),
    (
        true,
        "{#each foo. as x}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            43,
        )),
    ),
    (true, "{#each items as item}{/each}", None),
    (true, "{#each items as item, i (item.id)}{/each}", None),
    (
        true,
        "{#each items as unknown as number[] as item}{/each}",
        None,
    ),
    (true, "{#each a, i}{/each}", None),
    (true, "{#each a, i (i)}{/each}", None),
    (true, "{#each items as { y = z }}{/each}", None),
    (true, "{#each Array.from({ length: 10 }), i}{/each}", None),
    (true, "{#each [] as {a = 1}, i (a)}{/each}", None),
    (true, "{#each [] as [a, b]: [number, string]}{/each}", None),
    (true, "{#each [] as item: string, i (item)}{/each}", None),
    (true, "{#each f(x as any) as q}{/each}", None),
    (true, "{#each (a as any) as q}{/each}", None),
    (true, "{#each a as b, c}{/each}", None),
    (true, "{#each c ? x : y as T}{/each}", None),
    (
        true,
        "{#each a, b as item}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            40,
        )),
    ),
    (true, "{#each [] as q (q.id)}{/each}", None),
    (true, "{#each [] as {a: {b}}}{/each}", None),
    (true, "{#each [] as q: Map<string, number>}{/each}", None),
    (true, "{#each [] as q: (a: string) => void, i}{/each}", None),
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
