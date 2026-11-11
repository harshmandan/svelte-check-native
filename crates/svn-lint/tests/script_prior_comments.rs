//! The compiler parses every JavaScript comment it meets into one list
//! (`parser.root.comments`): comments in template expressions, in start
//! tags and in `<script>` bodies, in source order. A `<script>` is
//! parsed with that whole list, so every comment read before the
//! script's own content is attached as a leading comment of the
//! script's first node — a `svelte-ignore` written in a template
//! expression above the instance script silences that node. HTML
//! comments and `<style>` comments are not JavaScript and are not in
//! the list. Each case here was measured against svelte 5.57.2.

use std::path::Path;
use svn_lint::{CompatFeatures, Warning};

fn lint(source: &str) -> Vec<Warning> {
    svn_lint::lint_file(
        source,
        Path::new("t.svelte"),
        Some(true),
        CompatFeatures::MODERN,
    )
}

fn codes(source: &str) -> Vec<&'static str> {
    lint(source).iter().map(|w| w.code.as_str()).collect()
}

const IGNORE: &str = "/* svelte-ignore state_referenced_locally */";
const SCRIPT: &str = "<script>let count = $state(0), b = count; count++;</script>";

fn silenced(before: &str) {
    let src = format!("{before}\n{SCRIPT}");
    assert!(codes(&src).is_empty(), "{src}: {:?}", codes(&src));
}

fn warned(src: &str) {
    assert_eq!(codes(src), ["state_referenced_locally"], "{src}");
}

#[test]
fn control_warns() {
    warned(&format!("{{b}}\n{SCRIPT}"));
}

#[test]
fn comment_in_a_mustache_before_the_script() {
    silenced(&format!("{{{IGNORE} b}}"));
    silenced("{b // svelte-ignore state_referenced_locally\n}");
}

#[test]
fn comment_in_an_attribute_expression() {
    silenced(&format!("<div title={{{IGNORE} b}}></div>"));
    silenced(&format!("<div title=\"{{{IGNORE} b}}\"></div>"));
    silenced(&format!("<input bind:value={{{IGNORE} count}}>"));
    silenced(&format!("<div style:color={{{IGNORE} 'red'}}></div>"));
    silenced(&format!("<div {{...{IGNORE} {{}}}}></div>"));
}

#[test]
fn comment_in_a_start_tag() {
    silenced(&format!("<div {IGNORE}></div>"));
    silenced("<div // svelte-ignore state_referenced_locally\n></div>");
}

#[test]
fn comment_in_a_block_or_tag() {
    silenced(&format!("{{#if {IGNORE} b}}{{/if}}"));
    silenced(&format!("{{#each [{IGNORE}] as q}}{{q}}{{/each}}"));
    silenced(&format!("{{#each [] as q (q {IGNORE})}}{{q}}{{/each}}"));
    silenced(&format!("{{#await {IGNORE} null}}{{/await}}"));
    silenced(&format!("{{#key {IGNORE} 1}}{{/key}}"));
    silenced(&format!("{{@html {IGNORE} 'a'}}"));
    silenced(&format!("{{#snippet s({IGNORE} x)}}{{x}}{{/snippet}}"));
    silenced(&format!(
        "{{#if true}}{{@const z = {IGNORE} 1}}{{z}}{{/if}}"
    ));
}

#[test]
fn comment_in_an_earlier_module_script() {
    silenced(
        "<script module>// svelte-ignore state_referenced_locally\nexport const q = 1;</script>",
    );
    silenced(
        "<script module>export const q = 1; // svelte-ignore state_referenced_locally\n</script>",
    );
}

#[test]
fn only_the_first_node_is_silenced() {
    warned(&format!(
        "{{{IGNORE} b}}\n<script>let count = $state(0);\nlet b = count; count++;</script>"
    ));
    warned(&format!(
        "{{{IGNORE} b}}\n<script>'use strict';\nlet count = $state(0), b = count; count++;</script>"
    ));
}

#[test]
fn comments_after_the_script_or_outside_javascript_do_not_count() {
    warned(&format!("{SCRIPT}\n{{{IGNORE} b}}"));
    warned(&format!(
        "<style>{IGNORE} div{{}}</style>\n{SCRIPT}<div></div>"
    ));
    warned(&format!("<p>{IGNORE}</p>\n{SCRIPT}"));
    warned(&format!("{{'{IGNORE}'}}\n{SCRIPT}"));
    warned(&format!(
        "{SCRIPT}\n<script module>// svelte-ignore state_referenced_locally\nexport const q = 1;</script>"
    ));
}
