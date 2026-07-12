//! a11y rule-gate parity with the upstream compiler
//! (`phases/2-analyze/visitors/shared/a11y/index.js`).

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

fn codes(warnings: &[Warning]) -> Vec<&str> {
    warnings.iter().map(|w| w.code.as_str()).collect()
}

// ----------------------------------------------------------------
// role-supports-aria-props: a present-but-dynamic role disables the
// check (upstream: `role ? role_static_value : implicit_role`)
// ----------------------------------------------------------------

/// Upstream computes `role_value = role ? get_static_value(role) :
/// get_implicit_role(...)`. A role attribute that is present but
/// dynamic resolves to null — the implicit role must NOT be used as a
/// fallback, so role-supports-aria-props is skipped entirely.
#[test]
fn dynamic_role_disables_role_supports_aria_props() {
    let src = r#"<img src="x" alt="x" role={foo} aria-sort="ascending" />"#;
    let warnings = lint(src);
    let cs = codes(&warnings);
    assert!(
        !cs.contains(&"a11y_role_supports_aria_props")
            && !cs.contains(&"a11y_role_supports_aria_props_implicit"),
        "a dynamic role attribute must disable role-supports-aria-props, got: {cs:?}"
    );
}

/// Sanity: without a role attribute, the implicit role drives the
/// check and fires the `_implicit` variant.
#[test]
fn implicit_role_still_fires_role_supports_aria_props() {
    let src = r#"<img src="x" alt="x" aria-sort="ascending" />"#;
    let warnings = lint(src);
    assert!(
        codes(&warnings).contains(&"a11y_role_supports_aria_props_implicit"),
        "implicit img role does not support aria-sort, got: {:?}",
        codes(&warnings)
    );
}

/// Sanity: a static role attribute drives the non-implicit variant.
#[test]
fn static_role_still_fires_role_supports_aria_props() {
    let src = r#"<div role="article" aria-sort="ascending"></div>"#;
    let warnings = lint(src);
    assert!(
        codes(&warnings).contains(&"a11y_role_supports_aria_props"),
        "role article does not support aria-sort, got: {:?}",
        codes(&warnings)
    );
}
