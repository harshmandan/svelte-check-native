//! A closing tag that matches no open element.
//!
//! The compiler pops open elements until one has the closing tag's
//! name. When it reaches a block, a component or the root first, it
//! throws `element_invalid_closing_tag` at the tag. If a later open tag
//! had already closed an element of that name (`<p><div>` closes the
//! `<p>`), it throws `element_invalid_closing_tag_autoclosed` instead and
//! names that tag. Each case below is what svelte 5.57.2's `parse()`
//! reports for the template: code, offset, first line of the message.

use svn_parser::{ParseError, parse_template};

const CASES: &[(&str, &str, u32, &str)] = &[
    (
        "<p><div></div></p>",
        "element_invalid_closing_tag_autoclosed",
        14,
        "`</p>` attempted to close element that was already automatically closed by `<div>` (cannot nest `<div>` inside `<p>`)",
    ),
    (
        "<p><div></div></p>\n{a b}",
        "element_invalid_closing_tag_autoclosed",
        14,
        "`</p>` attempted to close element that was already automatically closed by `<div>` (cannot nest `<div>` inside `<p>`)",
    ),
    (
        "<div></p>",
        "element_invalid_closing_tag",
        5,
        "`</p>` attempted to close an element that was not open",
    ),
    (
        "<li><li></li></li>",
        "element_invalid_closing_tag_autoclosed",
        13,
        "`</li>` attempted to close element that was already automatically closed by `<li>` (cannot nest `<li>` inside `<li>`)",
    ),
    (
        "<p><div></div></x>",
        "element_invalid_closing_tag",
        14,
        "`</x>` attempted to close an element that was not open",
    ),
    (
        "<p><p></p></p>",
        "element_invalid_closing_tag_autoclosed",
        10,
        "`</p>` attempted to close element that was already automatically closed by `<p>` (cannot nest `<p>` inside `<p>`)",
    ),
    (
        "<div><p><div></div></p></div>",
        "element_invalid_closing_tag_autoclosed",
        19,
        "`</p>` attempted to close element that was already automatically closed by `<div>` (cannot nest `<div>` inside `<p>`)",
    ),
    (
        "{#if a}<p><div></div></p>{/if}",
        "element_invalid_closing_tag_autoclosed",
        21,
        "`</p>` attempted to close element that was already automatically closed by `<div>` (cannot nest `<div>` inside `<p>`)",
    ),
    (
        "<p><div></div>{#if a}</p>{/if}",
        "element_invalid_closing_tag_autoclosed",
        21,
        "`</p>` attempted to close element that was already automatically closed by `<div>` (cannot nest `<div>` inside `<p>`)",
    ),
    (
        "<p><div></div><span></p></span>",
        "element_invalid_closing_tag_autoclosed",
        20,
        "`</p>` attempted to close element that was already automatically closed by `<div>` (cannot nest `<div>` inside `<p>`)",
    ),
    (
        "<li><li></li><p></li>",
        "element_invalid_closing_tag_autoclosed",
        16,
        "`</li>` attempted to close element that was already automatically closed by `<li>` (cannot nest `<li>` inside `<li>`)",
    ),
    (
        "</p>",
        "element_invalid_closing_tag",
        0,
        "`</p>` attempted to close an element that was not open",
    ),
    (
        "<div></span></div>",
        "element_invalid_closing_tag",
        5,
        "`</span>` attempted to close an element that was not open",
    ),
    (
        "<Foo><p><div></div></p></Foo>",
        "element_invalid_closing_tag_autoclosed",
        19,
        "`</p>` attempted to close element that was already automatically closed by `<div>` (cannot nest `<div>` inside `<p>`)",
    ),
    (
        "<p><div></div></p></p>",
        "element_invalid_closing_tag_autoclosed",
        14,
        "`</p>` attempted to close element that was already automatically closed by `<div>` (cannot nest `<div>` inside `<p>`)",
    ),
];

#[test]
fn stray_closing_tags_carry_the_compilers_error() {
    let mut failures = Vec::new();
    for &(src, code, at, message) in CASES {
        let (_, errors) = parse_template(src, svn_core::Range::new(0, src.len() as u32));
        let got = errors.first().map(|e| {
            let text = e.to_string();
            let first_line = text.lines().next().unwrap_or_default().to_string();
            (e.code_slug(), e.range().start, e.range().end, first_line)
        });
        let want = Some((code, at, at, message.to_string()));
        if got != want {
            failures.push(format!("{src:?}\n  got:  {got:?}\n  want: {want:?}"));
        }
        assert!(
            errors
                .iter()
                .all(|e| !matches!(e, ParseError::MalformedOpenTag { .. })),
            "{src:?}: {errors:?}"
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
