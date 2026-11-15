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

#[test]
fn a_comment_leading_parentheses_is_not_read() {
    // A template expression is read with its parentheses as nodes, and
    // they are removed after the comments are attached: a comment that
    // led `(…)` leads nothing.
    let src = "<svelte:options runes />\n\
               <script lang=\"ts\">let a = 1; let b = 2; let items = [1]; function f(v: number) { return v; }</script>\n\
               {/* svelte-ignore p1 */ (a)}\n\
               {(/* svelte-ignore p2 */ a)}\n\
               {a + /* svelte-ignore p3 */ (b)}\n\
               {a + (/* svelte-ignore p4 */ b)}\n\
               <div title={/* svelte-ignore p5 */ (a + b)}></div>\n\
               {#if /* svelte-ignore p6 */ ((a))}{/if}\n\
               {#each items as i (/* svelte-ignore p7 */ (i))}{i}{/each}\n\
               {[ /* svelte-ignore p8 */ (a), b]}\n\
               {f(/* svelte-ignore p9 */ (a))}\n\
               {#key /* svelte-ignore p10 */ (a)}{/key}\n\
               {@html /* svelte-ignore p11 */ (a)}\n\
               {#each items as i}{@const c = /* svelte-ignore p12 */ (i)}{c}{/each}\n";
    assert_eq!(positions(src), ["unknown_code 4:20", "unknown_code 6:24"]);
}

/// `{#each}` heads with a comment before the iterable, then three
/// `bind:` pairs; `lang` is spliced in.
fn each_heads_and_bind_pairs(lang: &str) -> String {
    format!(
        "<svelte:options runes />\n\
         <script{lang}>let items = [{{a: 1}}]; let x = $state(1);</script>\n\
         {{#each /* svelte-ignore b1 */ items as item}}{{item.a}}{{/each}}\n\
         {{#each /* svelte-ignore b2 */ items as item, i (i)}}{{item.a}}{{/each}}\n\
         {{#each /* svelte-ignore b3 */ items, i}}{{i}}{{/each}}\n\
         {{#each /* svelte-ignore b4 */ items as {{ a = 1 }}}}{{a}}{{/each}}\n\
         {{#each ( /* svelte-ignore b5 */ items) as item}}{{item.a}}{{/each}}\n\
         {{#each /* svelte-ignore b6 */ (items) as item}}{{item.a}}{{/each}}\n\
         <input bind:value={{/* svelte-ignore b7 */ () => x, (v) => x = v}} />\n\
         <input bind:value={{() => x /* svelte-ignore b8 */, (v) => x = v}} />\n\
         <input bind:value={{() => x, (v) => /* svelte-ignore b10 */ x = v}} />\n"
    )
}

#[test]
fn each_iterable_comments_lead_the_nodes_the_compiler_drops() {
    // Read as TypeScript, `items as item` is a cast and `items, i` a
    // sequence; with no `as` after either, the compiler keeps only the
    // iterable inside, and the comment that led the dropped node is
    // never read. `{ a = 1 }` fails as a type, so the iterable is read
    // again on its own: both reads queue the comment, which then leads
    // `items` twice. A bind pair is visited only through its two
    // bodies, so a comment before the getter or after the comma is not
    // read either.
    assert_eq!(
        positions(&each_heads_and_bind_pairs(" lang=\"ts\"")),
        [
            "unknown_code 6:25",
            "unknown_code 6:25",
            "unknown_code 7:27",
            "unknown_code 11:53"
        ]
    );
    // As JavaScript, the reads stop before `as`: nothing is dropped but
    // the sequence and the parentheses.
    assert_eq!(
        positions(&each_heads_and_bind_pairs("")),
        [
            "unknown_code 3:25",
            "unknown_code 4:25",
            "unknown_code 6:25",
            "unknown_code 7:27",
            "unknown_code 11:53"
        ]
    );
}

#[test]
fn a_retried_each_read_queues_its_comment_twice_for_a_later_script() {
    let src = "<svelte:options runes />\n\
               {#each /* svelte-ignore b4 */ items as { a = 1 }}{a}{/each}\n\
               <script lang=\"ts\">let items = [{a: 1}];</script>\n";
    assert_eq!(positions(src), ["unknown_code 2:25"; 4]);
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

#[test]
fn comments_in_every_kind_of_template_expression_are_read() {
    let src = "<svelte:options runes />\n<script>\n\tlet items = $state([]);\n</script>\n\
               {#each items as item, i (/* svelte-ignore foo_b */ item)}\n\
               \t{@const x = /* svelte-ignore foo_c */ item}\n\t<p>{x}</p>\n{/each}\n\
               {#snippet s(/* svelte-ignore foo_d */ a)}{a}{/snippet}\n\
               {#await /* svelte-ignore foo_e */ items then v}{v}{/await}\n\
               {#key /* svelte-ignore foo_g */ items}{/key}\n\
               {@html /* svelte-ignore foo_h */ items}\n\
               {@render s(/* svelte-ignore foo_i */ 1)}\n\
               <input bind:value={/* svelte-ignore foo_j */ items[0]} />\n\
               <p class:a={/* svelte-ignore foo_k */ items} style:color={/* svelte-ignore foo_l */ \"red\"} {.../* svelte-ignore foo_m */ items}></p>\n";
    // A keyed each block's body is visited before its key.
    assert_eq!(
        positions(src),
        [
            "unknown_code 6:31",
            "unknown_code 5:43",
            "unknown_code 9:30",
            "unknown_code 10:26",
            "unknown_code 11:24",
            "unknown_code 12:25",
            "unknown_code 13:29",
            "unknown_code 14:37",
            "unknown_code 15:30",
            "unknown_code 15:76",
            "unknown_code 15:113"
        ]
    );
}

#[test]
fn snippet_parameter_and_const_tag_comments_are_read_once() {
    let src = "<svelte:options runes />\n\
               {#snippet s(/* svelte-ignore foo_d */ a, b = /* svelte-ignore foo_e */ 1, { c = (/* svelte-ignore foo_f */ 2) })}{a}{/snippet}\n\
               {#snippet t(a /* svelte-ignore foo_g */, b)}{a}{/snippet}\n\
               {#if true}\n\t{@const x = /* svelte-ignore foo_c */ 1}\n\
               \t{@const y = /* svelte-ignore foo_h */ { a: 1 }}\n\t{x}{y}\n{/if}\n";
    assert_eq!(
        positions(src),
        [
            "unknown_code 2:30",
            "unknown_code 2:63",
            "unknown_code 2:99",
            "unknown_code 5:31",
            "unknown_code 6:31"
        ]
    );
}

fn exception(source: &str) -> Option<String> {
    svn_lint::lint_file_or_exception(
        source,
        Path::new("t.svelte"),
        svn_lint::LintOptions::default(),
        CompatFeatures::MODERN,
    )
    .1
}

/// The comment the parser keeps for a top-level `<script>` is handed to
/// the script's program without a position. In runes mode a code in it
/// that names no warning is warned about at that missing position, and
/// the compiler throws placing it.
#[test]
fn a_bad_code_before_a_script_crashes_the_compiler_in_runes_mode() {
    let crash = Some("An impossible situation occurred".to_string());
    for source in [
        "<svelte:options runes />\n<!-- svelte-ignore a11y-autofocus -->\n<script>let x = 1;</script>\n{x}\n",
        "<svelte:options runes />\n<p>x</p>\n<!-- svelte-ignore bogus -->\n<script>let x = 1;</script>{x}\n",
        // `<style>` is no node of the template: the script still gets
        // the comment.
        "<svelte:options runes /><!-- svelte-ignore bogus --><style>p{}</style>\n<script>let x=1</script>{x}\n",
    ] {
        assert_eq!(exception(source), crash, "{source}");
    }
    // `<svelte:options>` is still in the template while the parser reads
    // the script, so it ends the walk back to the comment.
    let options_between =
        "<!-- svelte-ignore bogus --><svelte:options runes /><script>let x=1</script>{x}\n";
    assert_eq!(exception(options_between), None);
    assert_eq!(positions(options_between), ["unknown_code 1:20"]);
}

/// Positions inside a comment's text count UTF-16 units of the text the
/// compiler was given, which may be shorter than the file's.
#[test]
fn bad_code_columns_count_utf16_units_of_the_text_read() {
    assert_eq!(
        positions(
            "<svelte:options runes />\n<script>\n        /* svelte-ignore a11y_autofocus,\n        \u{e9}\u{e9} bogus_code */\n        let x = 1;\n</script>{x}\n"
        ),
        ["unknown_code 4:4"]
    );
    assert_eq!(
        positions(
            "<svelte:options runes />\n<p>x</p>\n<!-- svelte-ignore css_unused_selector,\u{e9} bogus -->\n<style>p{color:red}</style>\n"
        ),
        ["unknown_code 3:38"]
    );
}

/// Text between the comment and the tag counts as white space once its
/// character references are decoded.
#[test]
fn decoded_white_space_keeps_the_comment_before_a_tag() {
    assert_eq!(
        positions(
            "<!-- svelte-ignore state_referenced_locally -->&#32;\n<script>let c = $state(0); const d = c;</script>{d}\n"
        ),
        Vec::<String>::new()
    );
    assert_eq!(
        positions(
            "<svelte:options runes />\n<p>x</p>\n<!-- svelte-ignore bogus -->&nbsp;\n<style>p{color:red}</style>\n"
        ),
        ["unknown_code 3:16"]
    );
}

/// A snippet's head is read as an arrow function from its type
/// parameters on. acorn-typescript gives the arrow its type parameters
/// after its body, so a comment among them leads the first parameter.
#[test]
fn comments_in_snippet_type_parameters_lead_the_first_parameter() {
    assert_eq!(
        positions(
            "<svelte:options runes /><script lang=\"ts\">let x=1;</script>\n{#snippet foo<T /* svelte-ignore bogus */>(a: T)}{a}{/snippet}\n{x}\n"
        ),
        ["unknown_code 2:34"]
    );
}
