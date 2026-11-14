//! acorn's errors for a regular expression literal: which patterns it
//! rejects, how it words the rejection, and that it comes before a
//! syntax error later in the same text.
//!
//! acorn checks a regular expression's flags and pattern as soon as it
//! has read the literal's last character, before it looks at the next
//! token. So `/a)/ + ;` is an invalid pattern, not an unexpected `;`.
//! In an attribute value the literal can run past the quote that ends
//! the value: `class="{/if}"></div>` holds the literal `/if}"></` with
//! the flags `div`, and the `v` flag makes its `}` an error.
//!
//! Its check is its own: it accepts a quantifier count of any size, and
//! its messages follow V8's ("Incomplete quantifier", "Lone quantifier
//! brackets", "Nothing to repeat").
//!
//! Each case is what svelte 5.57.2's `compile()` throws: code, first
//! line of the message, and the byte range it points at.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32, u32)>;

const CASES: &[(&str, Expected)] = &[
    (
        "<div class=\"{/if}\"></div>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /if}\"></: Lone quantifier brackets",
            14,
            14,
        )),
    ),
    (
        "<div class=\"{/if}\">\n</div>",
        Some(("js_parse_error", "Unterminated regular expression", 14, 14)),
    ),
    (
        "<p>{x + /a)/ + }</p>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            9,
            9,
        )),
    ),
    (
        "<p>{f(/[b-a]/, }</p>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[b-a]/: Range out of order in character class",
            7,
            7,
        )),
    ),
    (
        "<script>let a = /a)/; let = ;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a]/u; if (</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a]/: Lone quantifier brackets",
            17,
            17,
        )),
    ),
    (
        "<script>const r = /{/u; (</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /{/: Lone quantifier brackets",
            19,
            19,
        )),
    ),
    (
        "<script>let a = /a]/; (</script>",
        Some(("js_parse_error", "Unexpected token", 23, 23)),
    ),
    (
        "<script>let a = /a)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\k<x>/u;\nlet = 1;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\k<x>/: Invalid named capture referenced",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<n>a)(?<n>b)/; (</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<n>a)(?<n>b)/: Duplicate capture group name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = 1;\nlet b = /a)/ + ;\n</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            28,
            28,
        )),
    ),
    (
        "<script>let a = /a)/g, b = /(/; (</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            17,
            17,
        )),
    ),
    (
        "<input value=\"{/a)/.test(x) ? }\">",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            16,
            16,
        )),
    ),
    (
        "{#if /a)/.test(x) + }{/if}",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            6,
            6,
        )),
    ),
    (
        "{@const a = /a)/ + }",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            13,
            13,
        )),
    ),
    (
        "<textarea>{/a)/ + }</textarea>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            12,
            12,
        )),
    ),
    (
        "<script>let a = ; let b = /a)/;</script>",
        Some(("js_parse_error", "Unexpected token", 16, 16)),
    ),
    (
        "<script>let a = /a{99999999999999999999}/, b = /a{/u; (</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a{/: Incomplete quantifier",
            48,
            48,
        )),
    ),
    (
        "<script>let a = /x{2,1}/, b = /(/; (</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /x{2,1}/: numbers out of order in {} quantifier",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /ok/, b = /[b-a]/v + ;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[b-a]/: Range out of order in character class",
            27,
            27,
        )),
    ),
    (
        "<script>let a = 1 / 2 / /(?<n>a)(?<n>b)/.source + ;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<n>a)(?<n>b)/: Duplicate capture group name",
            25,
            25,
        )),
    ),
    (
        "<script>let a = /\\p{Script=Foo}/u; (</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\p{Script=Foo}/: Invalid property value",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a{/u\n(;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a{/: Incomplete quantifier",
            17,
            17,
        )),
    ),
    (
        "<script>const 💥 = /[\\d-a]/u.test(\"/\") ? : 1;</script>",
        Some(("js_parse_error", "Unexpected character '💥'", 14, 14)),
    ),
    (
        "<script lang=\"ts\">let a: number = /a)/ as ; </script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            35,
            35,
        )),
    ),
    (
        "<p>{a ? /+/ : }</p>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /+/: Nothing to repeat",
            9,
            9,
        )),
    ),
    (
        "<p>{[/(?ii:a)/, }</p>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?ii:a)/: Duplicate regular expression modifiers",
            6,
            6,
        )),
    ),
    (
        "<input value=\"{x / /(?<=a)*/u.y + }\">",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<=a)*/: Nothing to repeat",
            20,
            20,
        )),
    ),
    (
        "<p>{`${/a]/v}` + }</p>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a]/: Lone quantifier brackets",
            8,
            8,
        )),
    ),
    (
        "<script>let a = /[b-a]/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[b-a]/: Range out of order in character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[b-a]/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[b-a]/: Range out of order in character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[b-a]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[b-a]/: Range out of order in character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[\\d-a]/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[\\d-a]/: Invalid character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[a-\\d]/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[a-\\d]/: Invalid character class",
            17,
            17,
        )),
    ),
    ("<script>let a = /[\\d-a]/;</script>", None),
    ("<script>let a = /[a--b]/v;</script>", None),
    (
        "<script>let a = /[a&&&b]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[a&&&b]/: Invalid character in character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[]]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[]]/: Lone quantifier brackets",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[(]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[(]/: Invalid character in character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[a-z&&]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[a-z&&]/: Unterminated character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[\\q{a]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[\\q{a]/: Invalid character in character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[^\\q{ab}]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[^\\q{ab}]/: Negated character class may contain strings",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[^\\p{RGI_Emoji}]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[^\\p{RGI_Emoji}]/: Negated character class may contain strings",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\p{RGI_Emoji}/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\p{RGI_Emoji}/: Invalid property name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\p{Foo}/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\p{Foo}/: Invalid property name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\p{Script=Foo}/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\p{Script=Foo}/: Invalid property value",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\p/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\p/: Invalid property name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\P{/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\P{/: Invalid property name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a{2,1}/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a{2,1}/: numbers out of order in {} quantifier",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a{2,1}/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a{2,1}/: numbers out of order in {} quantifier",
            17,
            17,
        )),
    ),
    ("<script>let a = /a{99999999999999999999}/;</script>", None),
    (
        "<script>let a = /\\u{110000}/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\u{110000}/: Invalid unicode escape",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\u{/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\u{/: Invalid unicode escape",
            17,
            17,
        )),
    ),
    ("<script>let a = /\\uD83D/u;</script>", None),
    (
        "<script>let a = /\\c/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\c/: Invalid unicode escape",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\-/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\-/: Invalid escape",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\1/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\1/: Invalid escape",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\2(a)/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\2(a)/: Invalid escape",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\k<a>/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\k<a>/: Invalid named capture referenced",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\k<a>(?<b>x)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\k<a>(?<b>x)/: Invalid named capture referenced",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<a>/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<a>/: Unterminated group",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<>a)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<>a)/: Invalid capture group name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<1a>a)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<1a>a)/: Invalid capture group name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<a>x)(?<a>y)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<a>x)(?<a>y)/: Duplicate capture group name",
            17,
            17,
        )),
    ),
    ("<script>let a = /(?<a>x)|(?<a>y)/;</script>", None),
    ("<script>let a = /(?i:a)/;</script>", None),
    (
        "<script>let a = /(?ii:a)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?ii:a)/: Duplicate regular expression modifiers",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?i-i:a)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?i-i:a)/: Duplicate regular expression modifiers",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?-:a)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?-:a)/: Invalid regular expression modifiers",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?x:a)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?x:a)/: Invalid group",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?:a/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?:a/: Unterminated group",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?=a/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?=a/: Unterminated group",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<=a/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<=a/: Unterminated group",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<!a/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<!a/: Unterminated group",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(a/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(a/: Unterminated group",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a]/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a]/: Lone quantifier brackets",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a}/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a}/: Lone quantifier brackets",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /{/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /{/: Lone quantifier brackets",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a{/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a{/: Incomplete quantifier",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a{1/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a{1/: Incomplete quantifier",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /*/;</script>",
        Some(("js_parse_error", "Unterminated comment", 16, 16)),
    ),
    (
        "<script>let a = /+/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /+/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /?/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /?/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a**/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a**/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a{1}{2}/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a{1}{2}/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?=a)*/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?=a)*/: Invalid quantifier",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<=a)*/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<=a)*/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /^*/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /^*/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\b+/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\b+/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[/;</script>",
        Some(("js_parse_error", "Unterminated regular expression", 17, 17)),
    ),
    (
        "<script>let a = /[a/v;</script>",
        Some(("js_parse_error", "Unterminated regular expression", 17, 17)),
    ),
    (
        "<script>let a = /[[a]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[[a]/: Unterminated character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\x/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\x/: Invalid escape",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\0\\1/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\0\\1/: Invalid escape",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<n>a)\\k<m>/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<n>a)\\k<m>/: Invalid named capture referenced",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /a|*/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a|*/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?:)|+/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?:)|+/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /x{1,}{2}/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /x{1,}{2}/: Nothing to repeat",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\u{12/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\u{12/: Invalid unicode escape",
            17,
            17,
        )),
    ),
    ("<script>let a = /[\\b-a]/;</script>", None),
    (
        "<script>let a = /[a-\\b]/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[a-\\b]/: Range out of order in character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\p{L/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\p{L/: Invalid property name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /(?<a\\u{12}>x)/;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<a\\u{12}>x)/: Invalid capture group name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\k/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\k/: Invalid named reference",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /\\k</u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\k</: Invalid capture group name",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[\\p{L}--]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[\\p{L}--]/: Invalid character in character class",
            17,
            17,
        )),
    ),
    (
        "<script>let a = /[a-z--\\q{}]/v;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /[a-z--\\q{}]/: Unterminated character class",
            17,
            17,
        )),
    ),
    ("<script>let a = /(?<𝒜>.)/;</script>", None),
    (
        "<script>let a = /\\u{0}{/u;</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /\\u{0}{/: Incomplete quantifier",
            17,
            17,
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
