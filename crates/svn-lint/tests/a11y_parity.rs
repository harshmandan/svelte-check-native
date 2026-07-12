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

// ----------------------------------------------------------------
// a11y_missing_content: labelled or contenteditable-bound empty
// headings are exempt (upstream `!is_labelled` +
// `!has_contenteditable_binding` gates)
// ----------------------------------------------------------------

/// An empty heading with aria-label is labelled — upstream's
/// `!is_labelled` gate suppresses a11y_missing_content.
#[test]
fn labelled_empty_heading_does_not_fire_missing_content() {
    for src in [
        r#"<h1 aria-label="Hello"></h1>"#,
        r#"<h1 aria-labelledby="other"></h1>"#,
        r#"<h1 title="Hello"></h1>"#,
    ] {
        let warnings = lint(src);
        assert!(
            !codes(&warnings).contains(&"a11y_missing_content"),
            "labelled heading must not fire a11y_missing_content for {src}, got: {:?}",
            codes(&warnings)
        );
    }
}

/// An empty heading whose content is supplied through a
/// contenteditable binding (bind:innerHTML / bind:textContent /
/// bind:innerText) is exempt.
#[test]
fn contenteditable_bound_heading_does_not_fire_missing_content() {
    let src = "<script>let x = $state('');</script>\n<h1 contenteditable bind:innerHTML={x}></h1>";
    let warnings = lint(src);
    assert!(
        !codes(&warnings).contains(&"a11y_missing_content"),
        "contenteditable-bound heading must not fire, got: {:?}",
        codes(&warnings)
    );
}

/// Sanity: a bare empty heading still fires.
#[test]
fn bare_empty_heading_fires_missing_content() {
    let warnings = lint("<h1></h1>");
    assert!(
        codes(&warnings).contains(&"a11y_missing_content"),
        "empty heading must fire a11y_missing_content, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// a11y_consider_explicit_label: runs on every <a> / <button>,
// href-independent; `inert` only suppresses when statically present
// ----------------------------------------------------------------

/// Upstream's shared `case 'a': case 'button':` block runs the
/// explicit-label check before any href handling — an <a> without
/// href still gets it.
#[test]
fn empty_anchor_without_href_fires_consider_explicit_label() {
    let src = "<a onclick={() => 1}></a>";
    let warnings = lint(src);
    assert!(
        codes(&warnings).contains(&"a11y_consider_explicit_label"),
        "unlabelled empty <a> fires regardless of href, got: {:?}",
        codes(&warnings)
    );
}

/// A statically-present `inert` (bare or literal value) suppresses the
/// check; upstream tests `get_static_value(inert) !== null`.
#[test]
fn static_inert_suppresses_consider_explicit_label() {
    let warnings = lint("<button inert></button>");
    assert!(
        !codes(&warnings).contains(&"a11y_consider_explicit_label"),
        "bare inert suppresses the label check, got: {:?}",
        codes(&warnings)
    );
}

/// A dynamic `inert={expr}` resolves to null upstream and does NOT
/// suppress.
#[test]
fn dynamic_inert_does_not_suppress_consider_explicit_label() {
    let src = "<script>let x = $state(false);</script>\n<button inert={x}></button>";
    let warnings = lint(src);
    assert!(
        codes(&warnings).contains(&"a11y_consider_explicit_label"),
        "dynamic inert must not suppress the label check, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// a11y_aria_activedescendant_has_tabindex: gated on the schema-based
// element interactivity (upstream `!is_interactive`)
// ----------------------------------------------------------------

/// img and label are NON-interactive per the role / AX-object schemas
/// upstream consults, so aria-activedescendant without tabindex fires
/// on them.
#[test]
fn activedescendant_fires_on_schema_non_interactive_elements() {
    for src in [
        r#"<img aria-activedescendant="x" src="x" alt="y" />"#,
        r#"<label aria-activedescendant="x">hi <input /></label>"#,
        r#"<div aria-activedescendant="x"></div>"#,
    ] {
        let warnings = lint(src);
        assert!(
            codes(&warnings).contains(&"a11y_aria_activedescendant_has_tabindex"),
            "aria-activedescendant on a non-interactive element must fire for {src}, got: {:?}",
            codes(&warnings)
        );
    }
}

/// Interactive elements (per the same schemas) are exempt.
#[test]
fn activedescendant_exempts_schema_interactive_elements() {
    for src in [
        r#"<input aria-activedescendant="x" />"#,
        r#"<a href="/x" aria-activedescendant="x">y</a>"#,
    ] {
        let warnings = lint(src);
        assert!(
            !codes(&warnings).contains(&"a11y_aria_activedescendant_has_tabindex"),
            "interactive element must not fire for {src}, got: {:?}",
            codes(&warnings)
        );
    }
}

// ----------------------------------------------------------------
// Name-based attribute checks run on every attribute value shape:
// upstream's loop only skips non-Attribute nodes, so name={expr} and
// {shorthand} attributes reach the same checks as name="literal"
// ----------------------------------------------------------------

/// Each of these fires purely off the attribute NAME upstream — the
/// expression/shorthand value must not gate the check.
#[test]
fn name_based_checks_fire_on_expression_and_shorthand_attributes() {
    let cases: &[(&str, &str)] = &[
        ("<input autofocus={true} />", "a11y_autofocus"),
        ("<div accesskey={key}>x</div>", "a11y_accesskey"),
        ("<div {accesskey}>x</div>", "a11y_accesskey"),
        (
            "<div aria-foobar={1}>x</div>",
            "a11y_unknown_aria_attribute",
        ),
        ("<div scope={s}>x</div>", "a11y_misplaced_scope"),
        (
            "<div aria-activedescendant={x}>x</div>",
            "a11y_aria_activedescendant_has_tabindex",
        ),
    ];
    for (src, expected) in cases {
        let warnings = lint(src);
        assert!(
            codes(&warnings).contains(expected),
            "{expected} must fire for {src}, got: {:?}",
            codes(&warnings)
        );
    }
}

/// A dynamic tabindex on a non-interactive element resolves to null
/// upstream, which counts as "not known negative" — the
/// no-noninteractive-tabindex warning fires.
#[test]
fn dynamic_tabindex_fires_no_noninteractive_tabindex() {
    let src = "<div tabindex={i}>x</div>";
    let warnings = lint(src);
    assert!(
        codes(&warnings).contains(&"a11y_no_noninteractive_tabindex"),
        "dynamic tabindex on a div must fire, got: {:?}",
        codes(&warnings)
    );
}

/// Expression-valued aria-* attributes also reach the
/// role-supports-aria-props loop (the check is name-based; only the
/// role's support table matters).
#[test]
fn role_supports_aria_props_fires_on_expression_valued_aria_attr() {
    let src = r#"<div role="article" aria-checked={true}>x</div>"#;
    let warnings = lint(src);
    assert!(
        codes(&warnings).contains(&"a11y_role_supports_aria_props"),
        "aria-checked={{expr}} on role=article must fire, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// is_parent-style ancestor checks walk past Component / snippet
// boundaries and treat a <svelte:element> ancestor as "unknown, play
// it safe" (upstream is_parent, a11y/index.js)
// ----------------------------------------------------------------

/// A <figcaption> whose nearest RegularElement ancestor (skipping the
/// Component frame) is <figure> must not warn.
#[test]
fn figcaption_inside_component_under_figure_does_not_warn() {
    let src = r#"<figure><Foo><figcaption>hi</figcaption></Foo><img src="a" alt="b"/></figure>"#;
    let warnings = lint(src);
    assert!(
        !codes(&warnings).contains(&"a11y_figcaption_parent"),
        "component frames are skipped when resolving the figure parent, got: {:?}",
        codes(&warnings)
    );
}

/// Same through a snippet boundary.
#[test]
fn figcaption_inside_snippet_under_figure_does_not_warn() {
    let src = "<figure>{#snippet s()}<figcaption>c</figcaption>{/snippet}{@render s()}<img src=\"x\" alt=\"y\"/></figure>";
    let warnings = lint(src);
    assert!(
        !codes(&warnings).contains(&"a11y_figcaption_parent"),
        "snippet frames are skipped when resolving the figure parent, got: {:?}",
        codes(&warnings)
    );
}

/// A <svelte:element> ancestor could render as <figure> — upstream's
/// is_parent returns true for it, suppressing the warning.
#[test]
fn figcaption_inside_svelte_element_does_not_warn() {
    let src = "<script>let tag = $state('figure');</script>\n<svelte:element this={tag}><figcaption>x</figcaption></svelte:element>";
    let warnings = lint(src);
    assert!(
        !codes(&warnings).contains(&"a11y_figcaption_parent"),
        "a svelte:element ancestor plays it safe, got: {:?}",
        codes(&warnings)
    );
}

/// autofocus inside a Component under <dialog>: the dialog ancestor is
/// still visible past the component frame, so no warning.
#[test]
fn autofocus_inside_component_under_dialog_does_not_warn() {
    let src = r#"<dialog><Foo><input autofocus /></Foo></dialog>"#;
    let warnings = lint(src);
    assert!(
        !codes(&warnings).contains(&"a11y_autofocus"),
        "dialog ancestor suppresses autofocus past the component frame, got: {:?}",
        codes(&warnings)
    );
}

/// autofocus inside <svelte:element>: the ancestor may render as
/// <dialog>, so upstream plays it safe and does not warn.
#[test]
fn autofocus_inside_svelte_element_does_not_warn() {
    let src = "<script>let tag = $state('dialog');</script>\n<svelte:element this={tag}><input autofocus /></svelte:element>";
    let warnings = lint(src);
    assert!(
        !codes(&warnings).contains(&"a11y_autofocus"),
        "svelte:element ancestor plays it safe for autofocus, got: {:?}",
        codes(&warnings)
    );
}

/// Sanity: a figcaption whose nearest element ancestor is not figure
/// still warns, and a bare autofocus still warns.
#[test]
fn ancestor_negative_cases_still_warn() {
    let warnings = lint("<div><figcaption>x</figcaption></div>");
    assert!(
        codes(&warnings).contains(&"a11y_figcaption_parent"),
        "figcaption under div must warn, got: {:?}",
        codes(&warnings)
    );
    let warnings = lint("<input autofocus />");
    assert!(
        codes(&warnings).contains(&"a11y_autofocus"),
        "bare autofocus must warn, got: {:?}",
        codes(&warnings)
    );
    // The nearest RegularElement ancestor decides: a div between the
    // figure and the figcaption still warns even through a component.
    let warnings = lint("<figure><Foo><div><figcaption>x</figcaption></div></Foo></figure>");
    assert!(
        codes(&warnings).contains(&"a11y_figcaption_parent"),
        "div is the nearest element ancestor, got: {:?}",
        codes(&warnings)
    );
}
