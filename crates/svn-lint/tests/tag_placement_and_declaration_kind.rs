//! Errors the compiler's parser raises for a tag that stands where
//! only text and expressions may, and for a declaration tag that is no
//! `let` or `const`.
//!
//! - `block_invalid_placement` / `tag_invalid_placement`: a `{#…}`
//!   block or `{@…}` tag in an attribute value or inside a
//!   `<textarea>`, whose content is read as text and expressions.
//! - `declaration_tag_invalid_type`: `{var …}`, `{interface …}`,
//!   `{enum …}` by their keyword alone, and `{type A = B}` once the
//!   statement read there turns out to be a type alias. `type` is no
//!   reserved word, so `{type}` and `{type + 1}` stay expressions.
//!
//! Each case is what svelte 5.57.2's `compile()` throws: code, first
//! line of the message, and the byte range it points at.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32, u32)>;

const CASES: &[(&str, Expected)] = &[
    (
        "<script lang=\"ts\"></script>\n{type foo = boolean}",
        Some((
            "declaration_tag_invalid_type",
            "Declaration tags must be `let` or `const` declarations",
            29,
            47,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{type Foo<T> = T[]}",
        Some((
            "declaration_tag_invalid_type",
            "Declaration tags must be `let` or `const` declarations",
            29,
            46,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{ type foo = boolean }",
        Some((
            "declaration_tag_invalid_type",
            "Declaration tags must be `let` or `const` declarations",
            30,
            48,
        )),
    ),
    ("<script lang=\"ts\"></script>\n{type}", None),
    ("<script lang=\"ts\"></script>\n{type + 1}", None),
    ("<script lang=\"ts\"></script>\n{type .x}", None),
    ("<script lang=\"ts\"></script>\n{type ()}", None),
    ("<script lang=\"ts\"></script>\n{type in foo}", None),
    (
        "<script lang=\"ts\"></script>\n{type foo}",
        Some(("js_parse_error", "Unexpected token", 37, 37)),
    ),
    ("<script lang=\"ts\"></script>\n{type$}", None),
    ("<script lang=\"ts\"></script>\n{types}", None),
    (
        "{type foo = boolean}",
        Some(("js_parse_error", "Unexpected token", 6, 6)),
    ),
    ("{type}", None),
    (
        "{var$}",
        Some((
            "declaration_tag_invalid_type",
            "Declaration tags must be `let` or `const` declarations",
            1,
            4,
        )),
    ),
    (
        "{enum$}",
        Some((
            "declaration_tag_invalid_type",
            "Declaration tags must be `let` or `const` declarations",
            1,
            5,
        )),
    ),
    (
        "{interface$}",
        Some((
            "declaration_tag_invalid_type",
            "Declaration tags must be `let` or `const` declarations",
            1,
            10,
        )),
    ),
    ("{var_}", None),
    ("{vars}", None),
    (
        "{var x = 1}",
        Some((
            "declaration_tag_invalid_type",
            "Declaration tags must be `let` or `const` declarations",
            1,
            4,
        )),
    ),
    (
        "{enum}",
        Some((
            "declaration_tag_invalid_type",
            "Declaration tags must be `let` or `const` declarations",
            1,
            5,
        )),
    ),
    (
        "<textarea>{@html a}</textarea>",
        Some((
            "tag_invalid_placement",
            "{@html ...} tag cannot be inside <textarea>",
            10,
            10,
        )),
    ),
    (
        "<textarea>\n\t{#each a as b}{b}{/each}\n</textarea>",
        Some((
            "block_invalid_placement",
            "{#each ...} block cannot be inside <textarea>",
            12,
            12,
        )),
    ),
    (
        "<textarea>{#if a}x{/if}</textarea>",
        Some((
            "block_invalid_placement",
            "{#if ...} block cannot be inside <textarea>",
            10,
            10,
        )),
    ),
    (
        "<textarea>{@const a = 1}</textarea>",
        Some((
            "tag_invalid_placement",
            "{@const ...} tag cannot be inside <textarea>",
            10,
            10,
        )),
    ),
    (
        "<textarea>{@render a()}</textarea>",
        Some((
            "tag_invalid_placement",
            "{@render ...} tag cannot be inside <textarea>",
            10,
            10,
        )),
    ),
    ("<textarea>{a}</textarea>", None),
    (
        "<textarea>{:else}</textarea>",
        Some(("js_parse_error", "Unexpected token", 11, 11)),
    ),
    (
        "<textarea>x{#}</textarea>",
        Some((
            "block_invalid_placement",
            "{# ...} block cannot be inside <textarea>",
            11,
            11,
        )),
    ),
    (
        "<textarea>{a}{@debug a}</textarea>",
        Some((
            "tag_invalid_placement",
            "{@debug ...} tag cannot be inside <textarea>",
            13,
            13,
        )),
    ),
    (
        "<div style=\"{#if c}a{/if}\"></div>",
        Some((
            "block_invalid_placement",
            "{#if ...} block cannot be in attribute value",
            12,
            12,
        )),
    ),
    (
        "<div style=\"a{@html c}\"></div>",
        Some((
            "tag_invalid_placement",
            "{@html ...} tag cannot be in attribute value",
            13,
            13,
        )),
    ),
    (
        "<div style={#if c}></div>",
        Some((
            "block_invalid_placement",
            "{#if ...} block cannot be in attribute value",
            11,
            11,
        )),
    ),
    (
        "<div style=\"{#each a as b}{b}{/each}\" title=\"t\"></div>",
        Some((
            "block_invalid_placement",
            "{#each ...} block cannot be in attribute value",
            12,
            12,
        )),
    ),
    (
        "<Foo a=\"{#if c}x{/if}\" />",
        Some((
            "block_invalid_placement",
            "{#if ...} block cannot be in attribute value",
            8,
            8,
        )),
    ),
    (
        "<div style=\"a {:else} b\"></div>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<div a=\"{#if c}\">x</div>",
        Some((
            "block_invalid_placement",
            "{#if ...} block cannot be in attribute value",
            8,
            8,
        )),
    ),
    (
        "<div a=\"{#}\"></div>",
        Some((
            "block_invalid_placement",
            "{# ...} block cannot be in attribute value",
            8,
            8,
        )),
    ),
];

#[test]
fn lint_reports_the_compiler_error() {
    let mut failures = Vec::new();
    for &(src, expected) in CASES {
        let warnings =
            svn_lint::lint_file(src, Path::new("T.svelte"), None, CompatFeatures::MODERN);
        let got = warnings.iter().find(|w| w.is_error).map(|w| {
            (
                w.code.as_str(),
                w.message.lines().next().unwrap_or_default(),
                w.range.start,
                w.range.end,
            )
        });
        if got != expected {
            failures.push(format!("{src:?}\n  got:  {got:?}\n  want: {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
