//! A `svelte-ignore` comment in a script applies to the node acorn
//! attaches it to as a leading comment: the first node, in walk order,
//! starting after the comment — unless the comment was taken as a
//! trailing comment of the node before it. Which node that is follows
//! from the tree, not from the characters around the comment.

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

fn codes(body: &str) -> Vec<&'static str> {
    let source = format!("<script>\n\tlet count = $state(0);\n\t{body}\n\tcount++;\n</script>\n");
    lint(&source).iter().map(|w| w.code.as_str()).collect()
}

#[test]
fn comment_before_an_array_hole_leads_the_next_element() {
    let body = "const g = [/* svelte-ignore state_referenced_locally */ , count];";
    assert!(codes(body).is_empty(), "{:?}", codes(body));
}

#[test]
fn comment_after_a_regex_literal_trails_it() {
    let body = "const s = [/x/ /* svelte-ignore state_referenced_locally */, count];";
    assert_eq!(codes(body), ["state_referenced_locally"]);
}

#[test]
fn comment_after_the_last_element_trails_it() {
    let body = "const t = [count /* svelte-ignore state_referenced_locally */];";
    assert_eq!(codes(body), ["state_referenced_locally"]);
}

#[test]
fn comment_after_an_operand_trails_it() {
    let body = "const u = a /* svelte-ignore state_referenced_locally */ ?? count; var a;";
    assert_eq!(codes(body), ["state_referenced_locally"]);
}
