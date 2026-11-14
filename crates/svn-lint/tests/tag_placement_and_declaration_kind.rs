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
        "<textarea>{/if}</textarea>",
        Some(("js_parse_error", "Invalid regular expression flag", 12, 12)),
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
    // A declaration acorn refuses for an early error: a name declared
    // twice, a reserved word bound, a number or regular expression it
    // does not allow. In a JavaScript component, then a TypeScript one.
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let q = 1, q = 2}",
        Some((
            "js_parse_error",
            "Identifier 'q' has already been declared",
            57,
            57,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{const {q, q} = obj}",
        Some((
            "js_parse_error",
            "Identifier 'q' has already been declared",
            56,
            56,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let [q, q] = obj}",
        Some((
            "js_parse_error",
            "Identifier 'q' has already been declared",
            54,
            54,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{const q = w => { let w }}",
        Some((
            "js_parse_error",
            "Identifier 'w' has already been declared",
            67,
            67,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{const q = function(w, w) {}}",
        Some(("js_parse_error", "Argument name clash", 68, 68)),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let q = class { #a; #a }}",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            66,
            66,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let q = y.#z}",
        Some((
            "js_parse_error",
            "Private field '#z' must be declared in an enclosing class",
            56,
            56,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{const q = super.y}",
        Some(("js_parse_error", "'super' keyword outside a method", 56, 56)),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{const q = 08}",
        Some(("js_parse_error", "Invalid number", 56, 56)),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let q = \"\\08\"}",
        Some(("js_parse_error", "Octal literal in strict mode", 55, 55)),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let q = delete y}",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            54,
            54,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let yield = 1}",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 50, 50)),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let static = 1}",
        Some(("js_parse_error", "The keyword 'static' is reserved", 50, 50)),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let implements = 1}",
        Some((
            "js_parse_error",
            "The keyword 'implements' is reserved",
            50,
            50,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let let = 1}",
        Some(("js_parse_error", "The keyword 'let' is reserved", 50, 50)),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let eval = 1}",
        Some(("js_parse_error", "Binding eval in strict mode", 50, 50)),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let arguments = 1}",
        Some(("js_parse_error", "Binding arguments in strict mode", 50, 50)),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{const q = { __proto__: 1, __proto__: 2 }}",
        Some((
            "js_parse_error",
            "Redefinition of __proto__ property",
            72,
            72,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let q = /(?<n>.)(?<n>.)/}",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<n>.)(?<n>.)/: Duplicate capture group name",
            55,
            55,
        )),
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{let q = a + 1}{q}",
        None,
    ),
    (
        "<script>let obj = {}, y = 1, a = 1;</script>\n{const {x, y} = obj}{x}{y}",
        Some((
            "declaration_duplicate",
            "`y` has already been declared",
            56,
            57,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let q = 1, q = 2}",
        Some((
            "js_parse_error",
            "Identifier 'q' has already been declared",
            67,
            67,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{const {q, q} = obj}",
        Some((
            "js_parse_error",
            "Identifier 'q' has already been declared",
            66,
            66,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let [q, q] = obj}",
        Some((
            "js_parse_error",
            "Identifier 'q' has already been declared",
            64,
            64,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{const q = w => { let w }}",
        Some((
            "js_parse_error",
            "Identifier 'w' has already been declared",
            77,
            77,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{const q = function(w, w) {}}",
        Some(("js_parse_error", "Argument name clash", 78, 78)),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let q = class { #a; #a }}",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            76,
            76,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let q = y.#z}",
        Some((
            "js_parse_error",
            "Private field '#z' must be declared in an enclosing class",
            66,
            66,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{const q = super.y}",
        Some(("js_parse_error", "'super' keyword outside a method", 66, 66)),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{const q = 08}",
        Some(("js_parse_error", "Invalid number", 66, 66)),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let q = \"\\08\"}",
        Some(("js_parse_error", "Octal literal in strict mode", 65, 65)),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let q = delete y}",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            64,
            64,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let yield = 1}",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 60, 60)),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let static = 1}",
        Some(("js_parse_error", "The keyword 'static' is reserved", 60, 60)),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let implements = 1}",
        Some((
            "js_parse_error",
            "The keyword 'implements' is reserved",
            60,
            60,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let let = 1}",
        Some(("js_parse_error", "The keyword 'let' is reserved", 60, 60)),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let eval = 1}",
        Some(("js_parse_error", "Binding eval in strict mode", 60, 60)),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let arguments = 1}",
        Some(("js_parse_error", "Binding arguments in strict mode", 60, 60)),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{const q = { __proto__: 1, __proto__: 2 }}",
        Some((
            "js_parse_error",
            "Redefinition of __proto__ property",
            82,
            82,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let q = /(?<n>.)(?<n>.)/}",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<n>.)(?<n>.)/: Duplicate capture group name",
            65,
            65,
        )),
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{let q = a + 1}{q}",
        None,
    ),
    (
        "<script lang=\"ts\">let obj = {}, y = 1, a = 1;</script>\n{const {x, y} = obj}{x}{y}",
        Some((
            "declaration_duplicate",
            "`y` has already been declared",
            66,
            67,
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
