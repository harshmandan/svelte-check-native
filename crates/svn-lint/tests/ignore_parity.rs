//! `<!-- svelte-ignore -->` extraction parity with the upstream
//! compiler (`utils/extract_svelte_ignore.js` + the 2-analyze `_`
//! visitor's backward sibling walk).

use std::path::Path;
use svn_lint::{CompatFeatures, Warning};

fn lint_runes(source: &str) -> Vec<Warning> {
    svn_lint::lint_file(
        source,
        Path::new("t.svelte"),
        Some(true),
        CompatFeatures::MODERN,
    )
}

fn codes(warnings: &[Warning]) -> Vec<&str> {
    warnings.iter().map(|w| w.code.as_str()).collect()
}

// ----------------------------------------------------------------
// Backward sibling walk: Text siblings never stop the chain
// ----------------------------------------------------------------

/// Upstream's analyze-phase visitor walks backward over siblings with
/// `if (prev.type === 'Comment') {...} else if (prev.type !== 'Text')
/// break;` — ANY Text node (whitespace or not) continues the chain.
/// An ignore comment therefore still applies to an element separated
/// from it by prose text.
#[test]
fn ignore_comment_bridges_non_whitespace_text_sibling() {
    let src = "<!-- svelte-ignore a11y_missing_attribute -->\nsome text\n<img src=\"x\" />";
    let warnings = lint_runes(src);
    assert!(
        !codes(&warnings).contains(&"a11y_missing_attribute"),
        "ignore comment must bridge the intervening text node, got: {:?}",
        codes(&warnings)
    );
}

/// Sanity: without the ignore comment the same input fires.
#[test]
fn missing_attribute_fires_without_ignore_comment() {
    let src = "some text\n<img src=\"x\" />";
    let warnings = lint_runes(src);
    assert!(
        codes(&warnings).contains(&"a11y_missing_attribute"),
        "img without alt must fire a11y_missing_attribute, got: {:?}",
        codes(&warnings)
    );
}

/// A non-Text, non-Comment sibling still stops the backward walk.
#[test]
fn ignore_comment_does_not_bridge_element_sibling() {
    let src = "<!-- svelte-ignore a11y_missing_attribute -->\n<div>x</div>\n<img src=\"x\" />";
    let warnings = lint_runes(src);
    assert!(
        codes(&warnings).contains(&"a11y_missing_attribute"),
        "an element between the comment and the target stops the ignore chain, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// Runes-mode comma parsing: stop at the first token not immediately
// followed by a comma (everything after is prose)
// ----------------------------------------------------------------

/// Upstream iterates `/([\w$-]+)(,)?/gm` and breaks when a token is
/// not immediately followed by `,`. In prose like this only the FIRST
/// word is treated as a (unknown) code.
#[test]
fn runes_prose_comment_fires_unknown_code_once() {
    let src = "<!-- svelte-ignore this is prose, do not worry -->\n<div>x</div>";
    let warnings = lint_runes(src);
    let unknown: Vec<_> = warnings
        .iter()
        .filter(|w| w.code.as_str() == "unknown_code")
        .collect();
    assert_eq!(
        unknown.len(),
        1,
        "only the first prose word is parsed as a code, got: {:?}",
        codes(&warnings)
    );
    assert!(
        unknown[0].message.contains("`this`"),
        "the unknown_code warning names the first token, got: {}",
        unknown[0].message
    );
}

/// A space before the comma means the first token is NOT immediately
/// followed by `,` — parsing stops after it, so the second code never
/// enters the suppression list and its warning still fires.
#[test]
fn runes_space_before_comma_stops_code_list() {
    let src = "<!-- svelte-ignore a11y_missing_attribute , a11y_autofocus -->\n<img src=\"x\" autofocus />";
    let warnings = lint_runes(src);
    let cs = codes(&warnings);
    assert!(
        cs.contains(&"a11y_autofocus"),
        "a11y_autofocus is prose after the detached comma and must still fire, got: {cs:?}"
    );
    assert!(
        !cs.contains(&"a11y_missing_attribute"),
        "the first code is still suppressed, got: {cs:?}"
    );
}

/// With the comma adjacent to the first token, both codes suppress.
#[test]
fn runes_adjacent_comma_suppresses_both_codes() {
    let src = "<!-- svelte-ignore a11y_missing_attribute, a11y_autofocus -->\n<img src=\"x\" autofocus />";
    let warnings = lint_runes(src);
    let cs = codes(&warnings);
    assert!(
        !cs.contains(&"a11y_autofocus") && !cs.contains(&"a11y_missing_attribute"),
        "both comma-separated codes suppress, got: {cs:?}"
    );
}

/// The stop-at-prose rule also governs the emit side: after a token
/// without an adjacent comma, later words are prose and produce no
/// unknown_code warnings.
#[test]
fn runes_known_code_then_prose_fires_nothing() {
    let src = "<!-- svelte-ignore a11y_autofocus because we need it -->\n<img src=\"x\" alt=\"y\" autofocus />";
    let warnings = lint_runes(src);
    let cs = codes(&warnings);
    assert!(
        !cs.contains(&"unknown_code") && !cs.contains(&"a11y_autofocus"),
        "prose after a known code is neither a code nor a warning, got: {cs:?}"
    );
}
