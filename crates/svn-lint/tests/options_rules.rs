#![allow(clippy::expect_used)]

//! `<svelte:options>` attribute warnings — parity with the loop over
//! `root.options.attributes` at the end of upstream's analyze phase
//! (`phases/2-analyze/index.js`): `accessors` / `immutable` warn in
//! runes mode, `customElement` warns without the compile option.

use std::path::Path;
use svn_lint::{CompatFeatures, Warning};

fn lint(source: &str, runes: bool) -> Vec<Warning> {
    svn_lint::lint_file(
        source,
        Path::new("t.svelte"),
        Some(runes),
        CompatFeatures::MODERN,
    )
}

fn codes(warnings: &[Warning]) -> Vec<&str> {
    warnings.iter().map(|w| w.code.as_str()).collect()
}

/// `<svelte:options accessors>` in runes mode fires the deprecation —
/// upstream checks only the attribute NAME, so the bare form, a
/// truthy value and even `accessors={false}` all fire.
#[test]
fn accessors_option_fires_in_runes_mode() {
    for src in [
        "<svelte:options accessors />",
        "<svelte:options accessors={true} />",
        "<svelte:options accessors={false} />",
    ] {
        let warnings = lint(src, true);
        assert!(
            codes(&warnings).contains(&"options_deprecated_accessors"),
            "accessors option must warn in runes mode for {src}, got: {:?}",
            codes(&warnings)
        );
    }
}

/// Same for `<svelte:options immutable>`.
#[test]
fn immutable_option_fires_in_runes_mode() {
    for src in [
        "<svelte:options immutable />",
        "<svelte:options immutable={true} />",
    ] {
        let warnings = lint(src, true);
        assert!(
            codes(&warnings).contains(&"options_deprecated_immutable"),
            "immutable option must warn in runes mode for {src}, got: {:?}",
            codes(&warnings)
        );
    }
}

/// Neither warning exists outside runes mode (the options still work
/// on legacy components).
#[test]
fn deprecated_options_do_not_fire_in_legacy_mode() {
    let warnings = lint("<svelte:options accessors immutable />", false);
    let cs = codes(&warnings);
    assert!(
        !cs.contains(&"options_deprecated_accessors")
            && !cs.contains(&"options_deprecated_immutable"),
        "legacy mode keeps accessors/immutable warning-free, got: {cs:?}"
    );
}

/// The upstream loop visits the attributes in source order, so
/// `accessors` before `customElement` yields the deprecation first
/// and the missing-custom-element warning second.
#[test]
fn options_warnings_fire_in_attribute_order() {
    let src = r#"<svelte:options accessors customElement="my-el" />"#;
    let warnings = lint(src, true);
    let cs: Vec<&str> = codes(&warnings)
        .into_iter()
        .filter(|c| c.starts_with("options_"))
        .collect();
    assert_eq!(
        cs,
        vec![
            "options_deprecated_accessors",
            "options_missing_custom_element"
        ],
        "options warnings follow attribute order"
    );
}

/// The warning spans the whole attribute, like upstream's
/// `w.options_deprecated_accessors(attribute)`.
#[test]
fn accessors_warning_spans_the_attribute() {
    let src = "<svelte:options accessors />";
    let warnings = lint(src, true);
    let w = warnings
        .iter()
        .find(|w| w.code.as_str() == "options_deprecated_accessors")
        .expect("accessors warning fires");
    // `accessors` starts at column 16 (0-based) and is 9 bytes long.
    assert_eq!((w.start_line, w.start_column), (1, 16));
    assert_eq!((w.end_line, w.end_column), (1, 25));
}
