//! Focused tests for `element_implicitly_closed`.
//!
//! The rule lives in `rules/implicit_close.rs` and is a source-level
//! tag scanner (it doesn't go through the AST), because upstream emits
//! this warning during the *parse* phase and our own parser hard-errors
//! on the same inputs instead of recovering. The scanner tracks an
//! open-tag stack and fires in two shapes:
//!
//!   1. opening `<X>` while the innermost open regular-element parent
//!      has an HTML5 closing-tag-omitted rule against `<X>` (e.g.
//!      `<p><div>` auto-closes the `<p>`).
//!   2. closing `</X>` where `X` is a regular element and one or more
//!      non-matching regular ancestors sit between the stack top and
//!      the matching open.
//!
//! **Shape (2) must never fire when `X` is a Component, a custom
//! element, a `svelte:*` tag, or a name that isn't on the stack at
//! all** — upstream's parse-stack tracks those separately, and
//! "</Foo>" encountered with only regular-element ancestors open is
//! either a clean component close (upstream pops its own Component
//! frame) or an invalid-close *error* (different diagnostic code,
//! not ours). Treating those closes as regular-close would bogusly
//! unwind the regular-element stack and fire on ancestors that are
//! actually well-formed — a huge false-positive class on any
//! workspace that nests DOM elements inside Components.

use std::path::Path;

fn lint(source: &str) -> Vec<svn_lint::Warning> {
    svn_lint::lint_file(source, Path::new("t.svelte"), Some(true))
}

fn implicit_close_warnings(source: &str) -> Vec<&'static str> {
    lint(source)
        .into_iter()
        .filter(|w| w.code == svn_lint::Code::element_implicitly_closed)
        .map(|_| "element_implicitly_closed")
        .collect()
}

/// A component wrapping a well-formed `<div>` must not fire the
/// warning. This is the Component-wraps-DOM false-positive shape.
#[test]
fn component_wraps_well_formed_div() {
    let src = "<Component>\n  <div>\n    content\n  </div>\n</Component>\n";
    let warnings = implicit_close_warnings(src);
    assert!(
        warnings.is_empty(),
        "expected no element_implicitly_closed warnings, got: {warnings:?}"
    );
}

/// Two sibling components nested inside a `<div>` pair must not
/// fire. The minimal shape of the Component-wraps-regular case.
#[test]
fn two_divs_wrapping_a_component() {
    let src = "\
{#if cond}
  <div class=\"outer\">
    <div class=\"inner\">
      <SettingsControl title=\"Voice\">
        hello
      </SettingsControl>
    </div>
  </div>
{/if}
";
    let warnings = implicit_close_warnings(src);
    assert!(
        warnings.is_empty(),
        "expected no warnings, got: {warnings:?}"
    );
}

/// `<svelte:fragment>` closes must also not unwind the regular stack.
#[test]
fn svelte_fragment_close_does_not_unwind() {
    let src = "<svelte:fragment>\n  <div>x</div>\n</svelte:fragment>\n";
    let warnings = implicit_close_warnings(src);
    assert!(
        warnings.is_empty(),
        "expected no warnings, got: {warnings:?}"
    );
}

/// Custom element close (kebab-case) must not unwind either.
#[test]
fn custom_element_close_does_not_unwind() {
    let src = "<my-widget>\n  <div>x</div>\n</my-widget>\n";
    let warnings = implicit_close_warnings(src);
    assert!(
        warnings.is_empty(),
        "expected no warnings, got: {warnings:?}"
    );
}

/// Unmatched regular close with no matching open ancestor must not
/// fire the warning — upstream emits `element_invalid_closing_tag`
/// (an error) which isn't our business.
#[test]
fn unmatched_regular_close_no_match_on_stack() {
    // `<span>` wraps, then `</div>` is encountered without any open
    // `<div>` frame.
    let src = "<span>\n  text\n</div>\n";
    let warnings = implicit_close_warnings(src);
    assert!(
        warnings.is_empty(),
        "expected no warnings (invalid close is an error, not a warning), got: {warnings:?}"
    );
}

/// **Regression:** the genuine auto-close shape — `<p>` opening a
/// `<div>` — must still fire per HTML5 closing_tag_omitted rules.
#[test]
fn p_auto_closed_by_div_still_fires() {
    let src = "<p>\n  <div>inside</div>\n</p>\n";
    let warnings = implicit_close_warnings(src);
    assert_eq!(
        warnings.len(),
        1,
        "expected one element_implicitly_closed from `<p><div>`, got: {warnings:?}"
    );
}

/// **Regression:** real malformed regular-only case — `<div><span></div>`.
/// `</div>` with `<span>` still open *should* fire — the `<span>` is
/// being auto-closed by `</div>`.
#[test]
fn span_auto_closed_by_div_close_still_fires() {
    let src = "<div>\n  <span>text\n</div>\n";
    let warnings = implicit_close_warnings(src);
    assert_eq!(
        warnings.len(),
        1,
        "expected one warning for span auto-closed by </div>, got: {warnings:?}"
    );
}
