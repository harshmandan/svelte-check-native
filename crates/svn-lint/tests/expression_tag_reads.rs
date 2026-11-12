//! How the compiler reads the tags that hold one expression: `{expr}`,
//! `{#if}`, `{:else if}`, `{#key}`, `{@html}`, `{@render}`, `{@debug}` and
//! `{@const}`.
//!
//! It skips the whitespace after the keyword (required for all but
//! `{expr}` and `{@debug}`), has acorn read the longest expression that
//! starts there, skips whitespace again and expects `}`. Nothing looks
//! for the `}` first. So a second expression after the first is
//! `expected_token` at its start, not a syntax error inside one long
//! expression, and without TypeScript an `as` ends the expression. A
//! `{@const}` reads its pattern and an `=` before the expression. Each
//! case below is what svelte 5.57.2's `parse()` reports for the template.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32)>;

/// `(typescript, template, the compiler's error: code, message, offset)`.
/// Offsets count from the start of the file, script line included.
const CASES: &[(bool, &str, Expected)] = &[
    (
        false,
        "{a b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            21,
        )),
    ),
    (
        true,
        "{a b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            31,
        )),
    ),
    (
        false,
        "{}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{ }",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            20,
        )),
    ),
    (
        true,
        "{ }",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            30,
        )),
    ),
    (
        false,
        "{#if a b}x{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            25,
        )),
    ),
    (
        true,
        "{#if a b}x{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            35,
        )),
    ),
    (
        false,
        "{#if}x{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            22,
        )),
    ),
    (
        true,
        "{#if}x{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            32,
        )),
    ),
    (
        false,
        "{#if a}x{:else if b c}y{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            38,
        )),
    ),
    (
        true,
        "{#if a}x{:else if b c}y{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            48,
        )),
    ),
    (
        false,
        "{#key a b}x{/key}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            26,
        )),
    ),
    (
        true,
        "{#key a b}x{/key}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            36,
        )),
    ),
    (
        false,
        "{@html a b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            27,
        )),
    ),
    (
        true,
        "{@html a b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            37,
        )),
    ),
    (
        false,
        "{@render a() b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            31,
        )),
    ),
    (
        true,
        "{@render a() b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            41,
        )),
    ),
    (
        false,
        "{@debug a b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            28,
        )),
    ),
    (
        true,
        "{@debug a b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            38,
        )),
    ),
    (false, "{a /* c */ }", None),
    (true, "{a /* c */ }", None),
    (false, "{a // c\n}", None),
    (true, "{a // c\n}", None),
    (false, "{#if a /* c */}x{/if}", None),
    (true, "{#if a /* c */}x{/if}", None),
    (
        false,
        "{a \"}",
        Some((
            "js_parse_error",
            "Unterminated string constant\nhttps://svelte.dev/e/js_parse_error",
            21,
        )),
    ),
    (
        true,
        "{a \"}",
        Some((
            "js_parse_error",
            "Unterminated string constant\nhttps://svelte.dev/e/js_parse_error",
            31,
        )),
    ),
    (
        false,
        "{a",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            20,
        )),
    ),
    (
        true,
        "{a",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            30,
        )),
    ),
    (
        false,
        "{#if a",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            24,
        )),
    ),
    (
        true,
        "{#if a",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            34,
        )),
    ),
    (
        false,
        "{(a}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            21,
        )),
    ),
    (
        true,
        "{(a}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            31,
        )),
    ),
    (
        false,
        "{a)}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            20,
        )),
    ),
    (
        true,
        "{a)}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            30,
        )),
    ),
    (
        false,
        "{`${a}",
        Some((
            "js_parse_error",
            "Unterminated template literal\nhttps://svelte.dev/e/js_parse_error",
            24,
        )),
    ),
    (
        true,
        "{`${a}",
        Some((
            "js_parse_error",
            "Unterminated template literal\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{@html}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            24,
        )),
    ),
    (
        true,
        "{@html}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
    (
        false,
        "{@html }",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            25,
        )),
    ),
    (
        true,
        "{@html }",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        false,
        "{@render}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            26,
        )),
    ),
    (
        true,
        "{@render}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            36,
        )),
    ),
    (
        false,
        "{#key}x{/key}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            23,
        )),
    ),
    (
        true,
        "{#key}x{/key}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            33,
        )),
    ),
    (
        false,
        "{#if a}x{:else if}y{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            35,
        )),
    ),
    (
        true,
        "{#if a}x{:else if}y{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            45,
        )),
    ),
    (
        false,
        "{#if a}x{:else ifb}y{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            35,
        )),
    ),
    (
        true,
        "{#if a}x{:else ifb}y{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            45,
        )),
    ),
    (false, "{@debugx}", None),
    (true, "{@debugx}", None),
    (
        false,
        "{@debug a, 1}",
        Some((
            "debug_tag_invalid_arguments",
            "{@debug ...} arguments must be identifiers, not arbitrary expressions\nhttps://svelte.dev/e/debug_tag_invalid_arguments",
            29,
        )),
    ),
    (
        true,
        "{@debug a, 1}",
        Some((
            "debug_tag_invalid_arguments",
            "{@debug ...} arguments must be identifiers, not arbitrary expressions\nhttps://svelte.dev/e/debug_tag_invalid_arguments",
            39,
        )),
    ),
    (
        false,
        "{a as b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            21,
        )),
    ),
    (true, "{a as b}", None),
    (
        false,
        "{a satisfies }",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            21,
        )),
    ),
    (
        true,
        "{a satisfies }",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            41,
        )),
    ),
    (
        false,
        "{#if a as}x{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            25,
        )),
    ),
    (
        true,
        "{#if a as}x{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (
        false,
        "{a,}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            21,
        )),
    ),
    (
        true,
        "{a,}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            31,
        )),
    ),
    (
        false,
        "{a ? b}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            24,
        )),
    ),
    (
        true,
        "{a ? b}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{a = }",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            23,
        )),
    ),
    (
        true,
        "{a = }",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        false,
        "{1 = 2}",
        Some((
            "js_parse_error",
            "Assigning to rvalue\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{1 = 2}",
        Some((
            "js_parse_error",
            "Assigning to rvalue\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (false, "{a }}", None),
    (true, "{a }}", None),
    (
        false,
        "{{a}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            22,
        )),
    ),
    (
        true,
        "{{a}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            32,
        )),
    ),
    (
        false,
        "{a{}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            20,
        )),
    ),
    (
        true,
        "{a{}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            30,
        )),
    ),
    (false, "{@debug}", None),
    (true, "{@debug}", None),
    (false, "{@debug }", None),
    (true, "{@debug }", None),
    (false, "{@render a?.()}", None),
    (true, "{@render a?.()}", None),
    (false, "{ a }", None),
    (true, "{ a }", None),
    (false, "{ /* c */ a }", None),
    (true, "{ /* c */ a }", None),
    (false, "{(a)}", None),
    (true, "{(a)}", None),
    (false, "{ ( a ) /* x */ }", None),
    (true, "{ ( a ) /* x */ }", None),
    (
        false,
        "{a /* c }",
        Some((
            "js_parse_error",
            "Unterminated comment\nhttps://svelte.dev/e/js_parse_error",
            21,
        )),
    ),
    (
        true,
        "{a /* c }",
        Some((
            "js_parse_error",
            "Unterminated comment\nhttps://svelte.dev/e/js_parse_error",
            31,
        )),
    ),
    (
        false,
        "{a /}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            22,
        )),
    ),
    (
        true,
        "{a /}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (false, "{a ? {} : {}}", None),
    (true, "{a ? {} : {}}", None),
    (false, "{a}}", None),
    (true, "{a}}", None),
    (false, "{`}`}", None),
    (true, "{`}`}", None),
    (false, "{\"}\"}", None),
    (true, "{\"}\"}", None),
    (false, "{a /* } */}", None),
    (true, "{a /* } */}", None),
    (false, "{a // }\n}", None),
    (true, "{a // }\n}", None),
    (false, "{#if a // }\n}x{/if}", None),
    (true, "{#if a // }\n}x{/if}", None),
    (false, "{@html `${a}}`}", None),
    (true, "{@html `${a}}`}", None),
    (false, "{@render a({})}", None),
    (true, "{@render a({})}", None),
    (false, "{#key {a}}x{/key}", None),
    (true, "{#key {a}}x{/key}", None),
    (false, "{#if a}x{:else if {}}y{/if}", None),
    (true, "{#if a}x{:else if {}}y{/if}", None),
    (false, "{#if a}x{:else if b}}y{/if}", None),
    (true, "{#if a}x{:else if b}}y{/if}", None),
    (
        false,
        "{@debug a,}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            28,
        )),
    ),
    (
        true,
        "{@debug a,}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            38,
        )),
    ),
    (false, "{@debug (a)}", None),
    (true, "{@debug (a)}", None),
    (
        false,
        "{@debug a.b}",
        Some((
            "debug_tag_invalid_arguments",
            "{@debug ...} arguments must be identifiers, not arbitrary expressions\nhttps://svelte.dev/e/debug_tag_invalid_arguments",
            26,
        )),
    ),
    (
        true,
        "{@debug a.b}",
        Some((
            "debug_tag_invalid_arguments",
            "{@debug ...} arguments must be identifiers, not arbitrary expressions\nhttps://svelte.dev/e/debug_tag_invalid_arguments",
            36,
        )),
    ),
    (
        false,
        "{@render a}",
        Some((
            "render_tag_invalid_expression",
            "`{@render ...}` tags can only contain call expressions\nhttps://svelte.dev/e/render_tag_invalid_expression",
            27,
        )),
    ),
    (
        true,
        "{@render a}",
        Some((
            "render_tag_invalid_expression",
            "`{@render ...}` tags can only contain call expressions\nhttps://svelte.dev/e/render_tag_invalid_expression",
            37,
        )),
    ),
    (false, "{@render (a())}", None),
    (true, "{@render (a())}", None),
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
        true,
        "{@htmlx}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
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
        true,
        "{@renderx()}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            36,
        )),
    ),
    (
        false,
        "{#ifx}y{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            22,
        )),
    ),
    (
        true,
        "{#ifx}y{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            32,
        )),
    ),
    (
        false,
        "{a\n b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            22,
        )),
    ),
    (
        true,
        "{a\n b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            32,
        )),
    ),
    (
        false,
        "{a /re/}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            25,
        )),
    ),
    (
        true,
        "{a /re/}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (false, "{a < b > c}", None),
    (true, "{a < b > c}", None),
    (false, "{a<b>(c)}", None),
    (true, "{a<b>(c)}", None),
    (
        false,
        "{<T>a}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (true, "{<T>a}", None),
    (
        false,
        "{a!}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            20,
        )),
    ),
    (true, "{a!}", None),
    (
        false,
        "{a!.b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            20,
        )),
    ),
    (true, "{a!.b}", None),
    (
        false,
        "{x = 1 as const}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            25,
        )),
    ),
    (true, "{x = 1 as const}", None),
    (false, "{async () => {}}", None),
    (true, "{async () => {}}", None),
    (false, "{function(){ return } }", None),
    (true, "{function(){ return } }", None),
    (false, "{class {}}", None),
    (true, "{class {}}", None),
    (
        false,
        "{a => }",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            24,
        )),
    ),
    (
        true,
        "{a => }",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{...a}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            19,
        )),
    ),
    (
        true,
        "{...a}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            29,
        )),
    ),
    (
        false,
        "{@html ...a}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            25,
        )),
    ),
    (
        true,
        "{@html ...a}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        false,
        "{#if a}x{:else if b as}y{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            38,
        )),
    ),
    (
        true,
        "{#if a}x{:else if b as}y{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            50,
        )),
    ),
    (false, "{ // c\n a }", None),
    (true, "{ // c\n a }", None),
    (
        false,
        "{a b",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            21,
        )),
    ),
    (
        true,
        "{a b",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            31,
        )),
    ),
    (
        false,
        "{@html a",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            26,
        )),
    ),
    (
        true,
        "{@html a",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            36,
        )),
    ),
    (
        false,
        "{#key a b",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            26,
        )),
    ),
    (
        true,
        "{#key a b",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            36,
        )),
    ),
    (
        false,
        "{#if x}{@const a = 1 2}{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            39,
        )),
    ),
    (
        true,
        "{#if x}{@const a = 1 2}{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            49,
        )),
    ),
    (
        false,
        "{#if x}{@const}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            32,
        )),
    ),
    (
        true,
        "{#if x}{@const}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            42,
        )),
    ),
    (
        false,
        "{#if x}{@const a}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            34,
        )),
    ),
    (
        true,
        "{#if x}{@const a}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            44,
        )),
    ),
    (false, "{#if x}{@const {a} = b}{/if}", None),
    (true, "{#if x}{@const {a} = b}{/if}", None),
    (
        false,
        "{#if x}{@const a: number = 1}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            31,
        )),
    ),
    (true, "{#if x}{@const a: number = 1}{/if}", None),
    (false, "{#if x}{@const a = (1, 2)}{/if}", None),
    (true, "{#if x}{@const a = (1, 2)}{/if}", None),
    (
        false,
        "{#if x}{@const a = 1, b = 2}{/if}",
        Some((
            "const_tag_invalid_expression",
            "{@const ...} must consist of a single variable declaration\nhttps://svelte.dev/e/const_tag_invalid_expression",
            37,
        )),
    ),
    (
        true,
        "{#if x}{@const a = 1, b = 2}{/if}",
        Some((
            "const_tag_invalid_expression",
            "{@const ...} must consist of a single variable declaration\nhttps://svelte.dev/e/const_tag_invalid_expression",
            47,
        )),
    ),
    (
        false,
        "{#if x}{@const a = b as c}{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            39,
        )),
    ),
    (true, "{#if x}{@const a = b as c}{/if}", None),
    (false, "{#if x}{@const [a, b] = c}{/if}", None),
    (true, "{#if x}{@const [a, b] = c}{/if}", None),
    (
        false,
        "{#if x}{@const {a] = c}{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            35,
        )),
    ),
    (
        true,
        "{#if x}{@const {a] = c}{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        false,
        "{#if x}{@const a = }{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (
        true,
        "{#if x}{@const a = }{/if}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            47,
        )),
    ),
    (
        false,
        "{#if x}{@const a /* c */ = 1 /* d */ }{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            35,
        )),
    ),
    (
        true,
        "{#if x}{@const a /* c */ = 1 /* d */ }{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (false, "{#if x}{@const a = 1}}{/if}", None),
    (true, "{#if x}{@const a = 1}}{/if}", None),
    (
        false,
        "{`a",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            20,
        )),
    ),
    (
        true,
        "{`a",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            30,
        )),
    ),
    (
        false,
        "{`a}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            20,
        )),
    ),
    (
        true,
        "{`a}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            30,
        )),
    ),
    (
        false,
        "{`${a}`",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            25,
        )),
    ),
    (
        true,
        "{`${a}`",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            35,
        )),
    ),
    (
        false,
        "{a + `b",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            24,
        )),
    ),
    (
        true,
        "{a + `b",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        false,
        "{`${`}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            23,
        )),
    ),
    (
        true,
        "{`${`}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        false,
        "{`${a",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            23,
        )),
    ),
    (
        true,
        "{`${a",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        false,
        "{`${a} b ${c}",
        Some((
            "js_parse_error",
            "Unterminated template literal\nhttps://svelte.dev/e/js_parse_error",
            31,
        )),
    ),
    (
        true,
        "{`${a} b ${c}",
        Some((
            "js_parse_error",
            "Unterminated template literal\nhttps://svelte.dev/e/js_parse_error",
            41,
        )),
    ),
    (false, "{`${a}`}", None),
    (true, "{`${a}`}", None),
    (
        false,
        "{#each `a as b}{/each}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            26,
        )),
    ),
    (
        true,
        "{#each `a as b}{/each}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        false,
        "{#each `${a} as b}{/each}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            30,
        )),
    ),
    (
        true,
        "{#each `${a} as b}{/each}",
        Some((
            "js_parse_error",
            "Unterminated template\nhttps://svelte.dev/e/js_parse_error",
            40,
        )),
    ),
    (false, "{a /* as */}", None),
    (true, "{a /* as */}", None),
    (false, "{a /* x as y */}", None),
    (true, "{a /* x as y */}", None),
    (false, "{a // as b\n}", None),
    (true, "{a // as b\n}", None),
    (false, "{f(a) /* as */ }", None),
    (true, "{f(a) /* as */ }", None),
    (
        false,
        "{a satisfies b}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            21,
        )),
    ),
    (true, "{a satisfies b}", None),
    (
        false,
        "{#eachx as y}{/each}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            24,
        )),
    ),
    (
        true,
        "{#eachx as y}{/each}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
    (
        false,
        "{#keyx}{/key}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            23,
        )),
    ),
    (
        true,
        "{#keyx}{/key}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            33,
        )),
    ),
    (
        false,
        "{#awaitx}{/await}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            25,
        )),
    ),
    (
        true,
        "{#awaitx}{/await}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            35,
        )),
    ),
    (
        false,
        "{#snippetx()}{/snippet}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            27,
        )),
    ),
    (
        true,
        "{#snippetx()}{/snippet}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            37,
        )),
    ),
    (
        false,
        "{#if(a)}y{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            22,
        )),
    ),
    (
        true,
        "{#if(a)}y{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            32,
        )),
    ),
    (
        false,
        "{#key(a)}y{/key}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            23,
        )),
    ),
    (
        true,
        "{#key(a)}y{/key}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            33,
        )),
    ),
    (
        false,
        "{@html(a)}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            24,
        )),
    ),
    (
        true,
        "{@html(a)}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
    (
        false,
        "{@html`a`}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            24,
        )),
    ),
    (
        true,
        "{@html`a`}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
    (false, "{@debug(a)}", None),
    (true, "{@debug(a)}", None),
    (
        false,
        "{@render(a())}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            26,
        )),
    ),
    (
        true,
        "{@render(a())}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            36,
        )),
    ),
    (
        false,
        "{#if a satisfies b}x{/if}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            25,
        )),
    ),
    (true, "{#if a satisfies b}x{/if}", None),
    (false, "{a /* satisfies */}", None),
    (true, "{a /* satisfies */}", None),
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
