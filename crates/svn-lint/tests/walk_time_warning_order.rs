//! The compiler raises `state_referenced_locally` when its script walk
//! visits the reference, so it lands among the warnings the same walk
//! raises at other nodes (`unknown_code`, `perf_avoid_nested_class`, …)
//! in the order the walk meets them. svelte-check prints a file's
//! compiler warnings in the order the compiler gives them.
//!
//! Every expectation here is the output of svelte@5.57.3's `compile`
//! for the same source (`code line:column`, column 1-based).

use std::path::Path;
use svn_lint::{CompatFeatures, Warning};

fn positions(source: &str) -> Vec<String> {
    let warnings: Vec<Warning> =
        svn_lint::lint_file(source, Path::new("t.svelte"), None, CompatFeatures::MODERN);
    warnings
        .iter()
        .map(|w| {
            format!(
                "{} {}:{}",
                w.code.as_str(),
                w.start_line,
                w.start_column + 1
            )
        })
        .collect()
}

#[test]
fn references_interleave_with_the_walks_other_warnings() {
    let src = "<svelte:options runes />\n<script>\n\tlet count = $state(0);\n\tlet a = count;\n\
               \t// svelte-ignore foo_x\n\tlet b = 1;\n\tlet c = count;\n\tfunction f() {\n\
               \t\tclass Inner {}\n\t\treturn count;\n\t}\n\
               \tlet d = { e: count, /* svelte-ignore foo_y */ g: count };\n</script>\n\
               <button onclick={() => count++}>{a}{b}{c}{d}{f}</button>\n";
    assert_eq!(
        positions(src),
        [
            "state_referenced_locally 4:10",
            "unknown_code 5:19",
            "state_referenced_locally 7:10",
            "perf_avoid_nested_class 9:3",
            "state_referenced_locally 12:15",
            "state_referenced_locally 12:51"
        ]
    );
}

#[test]
fn the_module_walk_comes_first() {
    let src = "<script module>\n\tlet m = 1;\n\t// svelte-ignore foo_m\n\tm;\n</script>\n\
               <svelte:options runes />\n<script>\n\tlet count = $state(0);\n\tlet a = count;\n\
               \t// svelte-ignore foo_i\n\tlet b = count;\n</script>\n\
               <button onclick={() => count++}>{a}{b}</button>\n";
    assert_eq!(
        positions(src),
        [
            "unknown_code 3:19",
            "unknown_code 3:19",
            "state_referenced_locally 9:10",
            "unknown_code 10:19",
            "state_referenced_locally 11:10"
        ]
    );
}
