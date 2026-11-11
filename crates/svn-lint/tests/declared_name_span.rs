//! Warnings reported on a declared name use the compiler's span for the
//! declarator's identifier. Svelte parses TypeScript with acorn's
//! TypeScript plugin, which makes a declarator's type annotation (and a
//! definite `!`) part of the Identifier node, so `x: number` is the
//! name's span. oxc keeps the annotation on the declarator instead.
//! Spans below were measured with `svelte/compiler`.

use std::path::Path;
use svn_lint::{CompatFeatures, Warning};

fn lint(source: &str, runes: Option<bool>) -> Vec<Warning> {
    svn_lint::lint_file(source, Path::new("t.svelte"), runes, CompatFeatures::MODERN)
}

/// `(code, text the warning covers)` for every warning.
fn spans(source: &str, runes: Option<bool>) -> Vec<(&'static str, String)> {
    lint(source, runes)
        .into_iter()
        .map(|w| {
            (
                w.code.as_str(),
                source[w.range.start as usize..w.range.end as usize].to_string(),
            )
        })
        .collect()
}

#[test]
fn export_let_unused_covers_the_type_annotation() {
    let cases = [
        (
            "<script lang=\"ts\">export let x: number = 0;</script>",
            vec!["x: number"],
        ),
        (
            "<script lang=\"ts\">export let x!: number;</script>",
            vec!["x!: number"],
        ),
        (
            "<script lang=\"ts\">export let x: { a: number } = { a: 1 };</script>",
            vec!["x: { a: number }"],
        ),
        (
            "<script lang=\"ts\">export let x: number = 0, y: string = '';</script>",
            vec!["x: number", "y: string"],
        ),
        ("<script lang=\"ts\">export let x;</script>", vec!["x"]),
        ("<script>export let x = 0;</script>", vec!["x"]),
    ];
    for (src, expected) in cases {
        let got: Vec<String> = spans(src, Some(false))
            .into_iter()
            .filter(|(code, _)| *code == "export_let_unused")
            .map(|(_, text)| text)
            .collect();
        assert_eq!(got, expected, "{src}");
    }
}

#[test]
fn custom_element_props_identifier_covers_the_type_annotation() {
    let src = "<svelte:options customElement=\"my-el\" />\n<script lang=\"ts\">\n\tlet props: { a: number } = $props()\n</script>\n\n<p>{props.a}</p>\n";
    let got: Vec<String> = spans(src, None)
        .into_iter()
        .filter(|(code, _)| *code == "custom_element_props_identifier")
        .map(|(_, text)| text)
        .collect();
    assert_eq!(got, ["props: { a: number }"]);
}
