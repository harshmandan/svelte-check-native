//! The compiler reads a top-level `<script>` / `<style>` tag's
//! attributes strictly (`read_static_attribute`): `=` right after the
//! name, a value after it, no repeated name, no `/>`. Where that
//! reading fails it throws, and nothing else comes out of the
//! component. Each case is the code and offset svelte 5.57.3's
//! `parse()` throws.

use std::path::Path;
use svn_lint::CompatFeatures;

const CASES: &[(&str, &str, u32)] = &[
    (
        "<script lang = \"ts\">let a = 1;</script>\n{b}",
        "expected_token",
        13,
    ),
    ("<script lang=\"ts\" />\n<p>{b}</p>", "expected_token", 18),
    (
        "<script lang=\"ts\" lang=\"ts\">let a = 1;</script>",
        "attribute_duplicate",
        18,
    ),
    (
        "<script a=>let a = 1;</script>",
        "expected_attribute_value",
        10,
    ),
    ("<script lang=\"ts\"\n\n", "unexpected_eof", 17),
    (
        "<script>let a = 1;</script>\n<style a b a>p{}</style>",
        "attribute_duplicate",
        39,
    ),
    // A template error the parser only meets after the tag: its
    // exception is shown at the file start but thrown later.
    (
        "<script lang=\"ts\"></script>\n<style a a></style>\n{#each a?.<T>b as x}x{/each}",
        "attribute_duplicate",
        37,
    ),
];

#[test]
fn a_script_tag_the_compiler_rejects_is_its_only_error() {
    let mut failures = Vec::new();
    for &(src, code, at) in CASES {
        let warnings =
            svn_lint::lint_file(src, Path::new("T.svelte"), None, CompatFeatures::MODERN);
        let got: Vec<_> = warnings
            .iter()
            .map(|w| (w.code.as_str(), w.range.start))
            .collect();
        if got != [(code, at)] {
            failures.push(format!(
                "{src:?}\n  got:  {got:?}\n  want: {:?}",
                [(code, at)]
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
