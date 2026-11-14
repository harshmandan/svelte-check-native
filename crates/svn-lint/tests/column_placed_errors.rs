//! acorn-typescript places a few errors (a class member's modifiers in
//! the wrong order, say) at a column number taken for an offset. acorn
//! counts the column from the last `\n` before where its read starts
//! and from every line break it reads after that; a `\r` before the
//! read is no line break to it (the compiler hands it a script with
//! everything before the script blanked to spaces, `\n` kept, or the
//! template itself). Each case is the offset svelte 5.57.3's `parse()`
//! throws `js_parse_error` at.

use std::path::Path;
use svn_lint::CompatFeatures;

const CASES: &[(&str, u32)] = &[
    // A template expression after a `\r`.
    (
        "<script lang=\"ts\"></script>\nab\r{(class { static public x = 1 })}",
        20,
    ),
    // The same with a `\n`: the column counts from it.
    (
        "<script lang=\"ts\"></script>\nab\n{(class { static public x = 1 })}",
        17,
    ),
    // A script after a `\r`.
    (
        "\r<script lang=\"ts\">class A { static public x = 1 }</script>",
        36,
    ),
    // An each block's expression after a `\r`.
    (
        "<script lang=\"ts\"></script>\nab\r{#each (class { static public x = 1 }) as a}{/each}",
        26,
    ),
];

#[test]
fn a_carriage_return_before_the_read_starts_no_line() {
    let mut failures = Vec::new();
    for &(src, at) in CASES {
        let warnings =
            svn_lint::lint_file(src, Path::new("T.svelte"), None, CompatFeatures::MODERN);
        let got: Vec<_> = warnings
            .iter()
            .filter(|w| w.is_error)
            .map(|w| (w.code.as_str(), w.range.start))
            .collect();
        if got != [("js_parse_error", at)] {
            failures.push(format!("{src:?}\n  got:  {got:?}\n  want: {at}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
