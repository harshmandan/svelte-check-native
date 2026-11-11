//! Focused tests for `element_implicitly_closed`.
//!
//! The parser decides when HTML ends an element early and records it on
//! the element; `rules/implicit_close.rs` reports what it recorded. Two
//! shapes fire:
//!
//!   1. opening `<X>` while the innermost open node is a plain element
//!      whose closing tag HTML lets `<X>` omit (`<p><div>` closes the
//!      `<p>`).
//!   2. closing `</X>` for an open ancestor while plain elements are
//!      still open inside it (`<div><span>a</div>`).
//!
//! A component, a `svelte:*` element or a block is never closed this
//! way, and while one is the innermost open node nothing outside it can
//! be closed either.

use std::path::Path;

fn lint(source: &str) -> Vec<svn_lint::Warning> {
    svn_lint::lint_file(
        source,
        Path::new("t.svelte"),
        Some(true),
        svn_lint::CompatFeatures::MODERN,
    )
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

/// A component is its own parent frame: an element opened among its
/// children never auto-closes an element outside the component. The
/// compiler only auto-closes when the *innermost* open node is a
/// regular element, and here that node is `<Wrapper>`.
#[test]
fn p_inside_component_inside_p_does_not_fire() {
    let src = "<p>\n\tBefore\n\t<Wrapper>\n\t\t<p>Inside the component</p>\n\t</Wrapper>\n\tafter.\n</p>\n";
    let warnings = implicit_close_warnings(src);
    assert!(
        warnings.is_empty(),
        "expected no warnings, got: {warnings:?}"
    );
}

/// Same for a `svelte:*` element between the two.
#[test]
fn div_inside_svelte_boundary_inside_p_does_not_fire() {
    let src = "<p>\n<svelte:boundary>\n<div>x</div>\n</svelte:boundary>\n</p>\n";
    let warnings = implicit_close_warnings(src);
    assert!(
        warnings.is_empty(),
        "expected no warnings, got: {warnings:?}"
    );
}

/// A custom element is a regular element with no omittable closing
/// tag, so it shields the outer `<p>` the same way.
#[test]
fn p_inside_custom_element_inside_p_does_not_fire() {
    let src = "<p>\n<my-el>\n<p>x</p>\n</my-el>\n</p>\n";
    let warnings = implicit_close_warnings(src);
    assert!(
        warnings.is_empty(),
        "expected no warnings, got: {warnings:?}"
    );
}

/// The warning spans from the closed element's start to its first
/// child (or to the trigger when it has none), as the compiler reports.
#[test]
fn warning_range_runs_to_first_child() {
    let src = "<ul><li>a<li>b</ul>";
    let ranges: Vec<_> = lint(src)
        .into_iter()
        .filter(|w| w.code == svn_lint::Code::element_implicitly_closed)
        .map(|w| (w.range.start, w.range.end, w.message))
        .collect();
    assert_eq!(ranges.len(), 2, "got: {ranges:?}");
    // First `<li>` closed by the second `<li>`: ends at its text child.
    assert_eq!((ranges[0].0, ranges[0].1), (4, 8));
    assert!(ranges[0].2.contains("`<li>`"), "{}", ranges[0].2);
    // Second `<li>` closed by `</ul>`.
    assert_eq!((ranges[1].0, ranges[1].1), (9, 13));
    assert!(ranges[1].2.contains("`</ul>`"), "{}", ranges[1].2);
}
