//! Where and how often the compiler reads a `svelte-ignore` comment.
//!
//! A JavaScript comment is attached by acorn to exactly one node — the
//! outermost one starting after it — and the analysis walk reads it
//! once, when it visits that node, applying its codes to everything
//! inside. Template expressions get their comments attached the same
//! way. An HTML comment is read by the next sibling that is not text or
//! a comment, and `<svelte:options>` is no sibling: the parser takes it
//! out of the template. The comment right before `<style>` is read once
//! more after the template, for `css_unused_selector`.
//!
//! Every expectation here is the output of svelte@5.57.3's `compile`
//! for the same source (`code line:column`, column 1-based).

use std::path::Path;
use svn_lint::{CompatFeatures, Warning};

fn lint(source: &str) -> Vec<Warning> {
    svn_lint::lint_file(source, Path::new("t.svelte"), None, CompatFeatures::MODERN)
}

fn positions(source: &str) -> Vec<String> {
    lint(source)
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

fn first_lines(source: &str) -> Vec<String> {
    lint(source)
        .iter()
        .map(|w| w.message.lines().next().unwrap_or_default().to_string())
        .collect()
}

// ----------------------------------------------------------------
// A script comment is read once, at the node it leads
// ----------------------------------------------------------------

#[test]
fn comment_before_an_assignment_statement_is_read_once() {
    let src =
        "<svelte:options runes />\n<script>\n// svelte-ignore unknown_thing\nx.y = 1;</script>\n";
    assert_eq!(positions(src), ["unknown_code 3:18"]);
}

#[test]
fn comments_before_calls_and_identifiers_are_read_once() {
    let src = "<svelte:options runes />\n<script>\n// svelte-ignore unknown_thing\nfoo();\n\
               /* svelte-ignore other_thing */ count;\n\
               // svelte-ignore non-top-level-reactive-declaration\nbar.baz();</script>\n";
    assert_eq!(
        positions(src),
        ["unknown_code 3:18", "unknown_code 5:18", "legacy_code 6:18"]
    );
}

#[test]
fn an_enclosing_ignore_of_unknown_code_silences_the_report() {
    let src = "<svelte:options runes />\n<script>\n// svelte-ignore unknown_code\nfunction f() {\n\t\
               // svelte-ignore foo_bar\n\tx;\n}\n</script>\n";
    assert!(positions(src).is_empty(), "{:?}", positions(src));
}

// ----------------------------------------------------------------
// A script comment applies to whatever node it leads
// ----------------------------------------------------------------

#[test]
fn the_unignored_reads_all_warn() {
    let src = "<script>\n\tlet count = $state(0);\n\tlet { a = count } = {};\n\t\
               switch (1) { case 1: count; }\n\tfor (let k = count;;) { break; }\n</script>\n";
    assert_eq!(
        positions(src),
        [
            "state_referenced_locally 3:12",
            "state_referenced_locally 4:23",
            "state_referenced_locally 5:15"
        ]
    );
}

#[test]
fn comment_leading_a_switch_case() {
    let src = "<script>\n\tlet count = $state(0);\n\tswitch (count) {\n\t\t\
               // svelte-ignore state_referenced_locally\n\t\tcase 1: count;\n\t}\n</script>\n";
    assert_eq!(positions(src), ["state_referenced_locally 3:10"]);
}

#[test]
fn comment_leading_a_destructuring_default() {
    let src = "<script>\n\tlet count = $state(0);\n\tconst obj = {};\n\t\
               let { /* svelte-ignore state_referenced_locally */ a = count } = obj;\n\t\
               let [/* svelte-ignore state_referenced_locally */ c = count] = [];\n</script>\n";
    assert!(positions(src).is_empty(), "{:?}", positions(src));
}

#[test]
fn comment_leading_a_catch_parameter_or_a_for_declaration() {
    let src = "<script>\n\tlet count = $state(0);\n\t\
               try {} catch (/* svelte-ignore state_referenced_locally */ { j = count }) {}\n\t\
               for (/* svelte-ignore state_referenced_locally */ let k = count;;) { break; }\n</script>\n";
    assert!(positions(src).is_empty(), "{:?}", positions(src));
}

// ----------------------------------------------------------------
// `<svelte:options>` is not part of the template
// ----------------------------------------------------------------

#[test]
fn options_do_not_end_the_comment_lookback() {
    let src = "<!-- svelte-ignore a11y_missing_attribute -->\n<svelte:options runes />\n<img src=\"x\">\n";
    assert!(positions(src).is_empty(), "{:?}", positions(src));
}

#[test]
fn options_do_not_read_the_comments_before_them() {
    let src = "<!-- svelte-ignore foo_one -->\n<svelte:options runes />\n\
               <!-- svelte-ignore foo_two -->\n<p></p>\n";
    assert_eq!(positions(src), ["unknown_code 3:20", "unknown_code 1:20"]);
}

// ----------------------------------------------------------------
// Comments in template expressions
// ----------------------------------------------------------------

#[test]
fn comments_in_template_expressions_are_read() {
    let src = "<svelte:options runes />\n{/* svelte-ignore foo_a */ x}\n\
               <div onclick={/* svelte-ignore foo_b */ () => {}}></div>\n\
               {#if /* svelte-ignore foo_c */ y}{/if}\n";
    assert_eq!(
        positions(src),
        [
            "unknown_code 2:19",
            "a11y_click_events_have_key_events 3:1",
            "a11y_no_static_element_interactions 3:1",
            "unknown_code 3:32",
            "unknown_code 4:23"
        ]
    );
}

#[test]
fn template_comments_are_read_again_before_a_later_script() {
    let src = "<svelte:options runes />\n{/* svelte-ignore foo_a */ x}\n\
               <div onclick={/* svelte-ignore foo_b */ () => {}}></div>\n\
               {#if /* svelte-ignore foo_c */ y}{/if}\n<script>\n\tlet x = 1, y = 2;\n</script>\n";
    assert_eq!(
        positions(src),
        [
            "unknown_code 2:19",
            "unknown_code 3:32",
            "unknown_code 4:23",
            "unknown_code 2:19",
            "a11y_click_events_have_key_events 3:1",
            "a11y_no_static_element_interactions 3:1",
            "unknown_code 3:32",
            "unknown_code 4:23"
        ]
    );
}

// ----------------------------------------------------------------
// A multi-line block comment loses its line's indentation first
// ----------------------------------------------------------------

#[test]
fn block_comment_lines_lose_the_indentation() {
    let src = "<svelte:options runes />\n<script>\n\t/*\n\t svelte-ignore baz_qux\n\t*/\n\tlet a = 1;\n</script>\n";
    assert_eq!(positions(src), ["unknown_code 4:16"]);
}

#[test]
fn block_comment_first_line_loses_the_indentation_too() {
    let src =
        "<svelte:options runes />\n<script>\n  /*  svelte-ignore x_y\n  */ let a = 1;\n</script>\n";
    assert_eq!(positions(src), ["unknown_code 3:19"]);
}

#[test]
fn script_tag_on_the_comment_line_counts_as_indentation() {
    let src = "<script>/*\n          svelte-ignore baz_qux\n*/\nlet a = 1;\n</script>\n<svelte:options runes />\n";
    assert_eq!(positions(src), ["unknown_code 2:17"]);
}

#[test]
fn template_block_comment_lines_lose_the_indentation() {
    let src =
        "<svelte:options runes />\n<div title={\n\t/*\n\t svelte-ignore tpl_code\n\t*/ 1}></div>\n";
    assert_eq!(positions(src), ["unknown_code 4:16"]);
}

// ----------------------------------------------------------------
// The comment before `<style>`
// ----------------------------------------------------------------

#[test]
fn comment_before_style_is_read_after_the_template() {
    let src = "<svelte:options runes />\n<div></div>\n<!-- svelte-ignore foo_qux -->\n\
               <style>\n\tdiv { color: red; }\n</style>\n<img>\n";
    assert_eq!(
        positions(src),
        [
            "unknown_code 3:20",
            "a11y_missing_attribute 7:1",
            "unknown_code 3:16"
        ]
    );
}

#[test]
fn comment_before_style_with_options_after() {
    let src = "<div></div>\n<!-- svelte-ignore foo_qux -->\n<style>\n\tdiv { color: red; }\n</style>\n\
               <!-- svelte-ignore bar_qux -->\n<svelte:options runes />\n";
    assert_eq!(positions(src), ["unknown_code 2:16"]);
}

// ----------------------------------------------------------------
// Suggestions follow the compiler's fuzzy matcher
// ----------------------------------------------------------------

#[test]
fn suggestion_ties_resolve_as_the_compiler_does() {
    let src = "<svelte:options runes />\n<script>\n// svelte-ignore a11y_misplaced_sribute\nlet a = 1;\n\
               // svelte-ignore a11y_incorrect_aria_attribute_type_toklean, foo-bar-baz, ssr-dom-renamed\n\
               let b = 2;\n</script>\n";
    assert_eq!(
        first_lines(src),
        [
            "`a11y_misplaced_sribute` is not a recognised code (did you mean `a11y_misplaced_scope`?)",
            "`a11y_incorrect_aria_attribute_type_toklean` is not a recognised code (did you mean `a11y_incorrect_aria_attribute_type_token`?)",
            "`foo-bar-baz` is not a recognised code",
            "`ssr-dom-renamed` is not a recognised code"
        ]
    );
}
