//! `{@attach …}` is its own node in the compiler's AST (`AttachTag`), not
//! a spread. Checks that look for a spread (`SpreadAttribute`) to give up
//! on what the element might carry must not count an attachment, and
//! elements that reject anything but attributes reject it too.

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

const SCRIPT: &str = "<script>let a = () => {};</script>";

#[test]
fn img_with_attachment_still_needs_alt() {
    let src = format!("{SCRIPT}<img src=\"x.png\" {{@attach a}}>");
    assert_eq!(codes(&src), ["a11y_missing_attribute"]);
}

#[test]
fn track_with_attachment_is_not_a_caption_track() {
    let src = format!("{SCRIPT}<video src=\"v.mp4\"><track {{@attach a}}></video>");
    assert_eq!(codes(&src), ["a11y_media_has_caption"]);
}

#[test]
fn slot_rejects_attachment() {
    let src = format!("{SCRIPT}<slot {{@attach a}}></slot>");
    assert_eq!(codes(&src), ["slot_element_invalid_attribute"]);
}

#[test]
fn svelte_window_accepts_attachment() {
    let src = format!("{SCRIPT}<svelte:window {{@attach a}} />");
    assert!(codes(&src).is_empty(), "{:?}", codes(&src));
}

#[test]
fn svelte_options_rejects_attachment() {
    let src = format!("{SCRIPT}<svelte:options {{@attach a}} />");
    assert_eq!(codes(&src), ["svelte_options_invalid_attribute"]);
}
