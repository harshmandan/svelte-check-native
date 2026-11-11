//! The compiler reads the keyword of a `{@…}` tag by prefix: it tries
//! `html`, `debug`, `const` and `render` in turn and takes the first
//! one the text after the `@` starts with, then reads that tag's own
//! syntax from right after the keyword. `{@constx a = 1}` is a
//! `{@const}` missing its whitespace, not an unknown tag, and
//! `{@debugx}` is a valid `{@debug x}`. Only text that starts with none
//! of the keywords is `expected_tag`. Each case below is what svelte
//! 5.57.2's `parse()` reports for the template.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32)>;

/// `(typescript, template, the compiler's error: code, message, offset)`.
/// Offsets count from the start of the file, script line included.
const CASES: &[(bool, &str, Expected)] = &[
    (
        false,
        "{#if 1}{@constx a = 1}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            32,
        )),
    ),
    (
        false,
        "{#if 1}{@constant}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            32,
        )),
    ),
    (
        false,
        "{@htmlx}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            24,
        )),
    ),
    (
        false,
        "{@htmlfoo bar}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            24,
        )),
    ),
    (false, "{@html x}", None),
    (
        false,
        "{@renderx()}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            26,
        )),
    ),
    (
        false,
        "{@renderfoo()}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            26,
        )),
    ),
    (false, "{@debugx}", None),
    (false, "{@debugger}", None),
    (false, "{@debug x}", None),
    (false, "{@debug}", None),
    (
        false,
        "{@ html x}",
        Some((
            "expected_tag",
            "Expected 'html', 'render', 'attach', 'const', or 'debug'\nhttps://svelte.dev/e/expected_tag",
            20,
        )),
    ),
    (
        false,
        "{@attachx}",
        Some((
            "expected_tag",
            "Expected 'html', 'render', 'attach', 'const', or 'debug'\nhttps://svelte.dev/e/expected_tag",
            20,
        )),
    ),
    (
        false,
        "{@foo}",
        Some((
            "expected_tag",
            "Expected 'html', 'render', 'attach', 'const', or 'debug'\nhttps://svelte.dev/e/expected_tag",
            20,
        )),
    ),
    (
        false,
        "{@HTML x}",
        Some((
            "expected_tag",
            "Expected 'html', 'render', 'attach', 'const', or 'debug'\nhttps://svelte.dev/e/expected_tag",
            20,
        )),
    ),
    (
        true,
        "{#if 1}{@constx a = 1}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            42,
        )),
    ),
    (
        true,
        "{#if 1}{@constant}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            42,
        )),
    ),
    (
        true,
        "{@htmlx}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
    (
        true,
        "{@htmlfoo bar}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
    (true, "{@html x}", None),
    (
        true,
        "{@renderx()}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            36,
        )),
    ),
    (
        true,
        "{@renderfoo()}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            36,
        )),
    ),
    (true, "{@debugx}", None),
    (true, "{@debugger}", None),
    (true, "{@debug x}", None),
    (true, "{@debug}", None),
    (
        true,
        "{@ html x}",
        Some((
            "expected_tag",
            "Expected 'html', 'render', 'attach', 'const', or 'debug'\nhttps://svelte.dev/e/expected_tag",
            30,
        )),
    ),
    (
        true,
        "{@attachx}",
        Some((
            "expected_tag",
            "Expected 'html', 'render', 'attach', 'const', or 'debug'\nhttps://svelte.dev/e/expected_tag",
            30,
        )),
    ),
    (
        true,
        "{@foo}",
        Some((
            "expected_tag",
            "Expected 'html', 'render', 'attach', 'const', or 'debug'\nhttps://svelte.dev/e/expected_tag",
            30,
        )),
    ),
    (
        true,
        "{@HTML x}",
        Some((
            "expected_tag",
            "Expected 'html', 'render', 'attach', 'const', or 'debug'\nhttps://svelte.dev/e/expected_tag",
            30,
        )),
    ),
    (
        false,
        "{@constant}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            25,
        )),
    ),
    (
        false,
        "<div>{@constx = 1}</div>",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            30,
        )),
    ),
    (
        true,
        "{@constant}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            35,
        )),
    ),
    (
        true,
        "<div>{@constx = 1}</div>",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            40,
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
