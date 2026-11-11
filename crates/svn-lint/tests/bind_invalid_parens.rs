//! `bind:value={get, set}` must not be wrapped in parentheses. The
//! compiler walks back from the start of the getter/setter sequence to
//! the `{` that opens the value and rejects any `(` it passes — unless
//! that `(` lies inside the comments written in front of the sequence,
//! which belong to the expression. A `{` inside such a comment is where
//! the walk stops, as it is for any other `{`.

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

const SCRIPT: &str = "<script>let v = $state(''); const get = () => v; const set = (x) => (v = x);</script>\n";

#[test]
fn parenthesis_in_a_leading_comment_is_allowed() {
    let src = format!("{SCRIPT}<input bind:value={{/* ( */ get, set}} />");
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}

#[test]
fn parenthesis_after_a_brace_in_a_leading_comment_is_allowed() {
    let src = format!("{SCRIPT}<input bind:value={{/* {{ ( */ get, set}} />");
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}

#[test]
fn surrounding_parenthesis_is_rejected() {
    let src = format!("{SCRIPT}<input bind:value={{( get, set)}} />");
    assert_eq!(codes(&src), ["bind_invalid_parens"]);
}

#[test]
fn parenthesis_after_a_leading_comment_is_rejected() {
    let src = format!("{SCRIPT}<input bind:value={{/* c */ (get, set)}} />");
    assert_eq!(codes(&src), ["bind_invalid_parens"]);
}
