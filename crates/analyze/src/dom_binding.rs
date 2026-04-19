//! Per-DOM-binding type table for one-way `bind:` directives.
//!
//! Two families of one-way binding are modeled here:
//!
//! 1. **Not-on-element**: ResizeObserver / HTMLMediaElement lists
//!    whose type lives on a SEPARATE browser API rather than on the
//!    bound element. `<div bind:contentRect={rect}>` doesn't assign
//!    `div.contentRect` — Svelte wires up a ResizeObserver behind
//!    the scenes and delivers `DOMRectReadOnly` into `rect`.
//!
//! 2. **Element-native** (v0.3 Item 4 — partial port of upstream's
//!    `oneWayBindingAttributes` table): properties that live
//!    directly on a DOM element type. Upstream emits `var =
//!    element.NAME` to run the assignment through tag-specific
//!    ambient types. We use tag-specific indexed access
//!    (`HTMLMediaElement['duration']`, `HTMLImageElement[
//!    'naturalWidth']`) so tsgo resolves the indexed access to
//!    the right primitive type without needing a per-tag dispatch
//!    table. The HTMLElement-layout subset (clientWidth etc.) is
//!    deferred — see the `type_for` body for the reason.
//!
//! For BOTH families, emit produces `__svn_any_as<TYPE>(expr)` — a
//! phantom contract call that type-checks `expr`'s inferred type
//! against `TYPE` without disturbing narrowing.
//!
//! Bidirectional bindings (`bind:value`, `bind:checked`, etc.) are
//! intentionally NOT here — those need read-AND-write flow and are
//! tracked in NEXT.md as deferred scope.

/// Return the TS type to assert for `bind:NAME`. `None` means the
/// binding isn't one we model (a typo like `bind:foo`, or the
/// bidirectional family we don't yet type-check).
///
/// Names taken verbatim from upstream svelte2tsx's
/// `oneWayBindingAttributesNotOnElement` map (not-on-element family)
/// and `oneWayBindingAttributes` set (element-native family) so
/// parity with upstream's type-check behavior is preserved.
pub fn type_for(binding_name: &str) -> Option<&'static str> {
    match binding_name {
        // --- Not-on-element family (shipped in v0.2) ----------------
        "contentRect" => Some("DOMRectReadOnly"),
        "contentBoxSize" => Some("ResizeObserverSize[]"),
        "borderBoxSize" => Some("ResizeObserverSize[]"),
        "devicePixelContentBoxSize" => Some("ResizeObserverSize[]"),
        // Media element lists — available on the element at runtime
        // but with a different type than what Svelte surfaces via
        // bind:. The Svelte-package-provided type is authoritative.
        "buffered" => Some("import('svelte/elements').SvelteMediaTimeRange[]"),
        "played" => Some("import('svelte/elements').SvelteMediaTimeRange[]"),
        "seekable" => Some("import('svelte/elements').SvelteMediaTimeRange[]"),

        // --- Element-native one-way family (v0.3 Item 4 + 6) --------
        //
        // Image dimensions — HTMLImageElement-specific.
        "naturalWidth" => Some("HTMLImageElement['naturalWidth']"),
        "naturalHeight" => Some("HTMLImageElement['naturalHeight']"),
        // Media playback — HTMLMediaElement (<audio>, <video>).
        "duration" => Some("HTMLMediaElement['duration']"),
        "seeking" => Some("HTMLMediaElement['seeking']"),
        "ended" => Some("HTMLMediaElement['ended']"),
        "readyState" => Some("HTMLMediaElement['readyState']"),
        // Layout measurements — on every HTMLElement. v0.3 Item 6
        // re-added these after moving the `__svn_any_as<...>(EXPR);`
        // emit INLINE at the bind-site inside the template walker
        // (see `emit_dom_binding_checks_inline` in
        // `emit_template_node`'s Node::Element arm). The top-of-
        // tpl_check batch placement surfaced `Cannot find name 'i'`
        // TS2304 noise whenever the expression referenced a
        // block-scoped iterator (`bind:clientWidth={items[i].width}`
        // inside `{#each as item, i}`); inline emit resolves those
        // bindings against the enclosing block's scope.
        "clientWidth" => Some("HTMLElement['clientWidth']"),
        "clientHeight" => Some("HTMLElement['clientHeight']"),
        "offsetWidth" => Some("HTMLElement['offsetWidth']"),
        "offsetHeight" => Some("HTMLElement['offsetHeight']"),

        // --- Bidirectional bindings (v0.3 Item 8 — narrow subset) ---
        //
        // `bind:checked` / `bind:files` on `<input>` have a FIXED
        // target type independent of the input's `type` attribute:
        //
        //   - `bind:checked` → boolean (for checkbox/radio; invalid
        //     on other input types, but svelte accepts at runtime)
        //   - `bind:files`   → FileList | null (only meaningful on
        //     type="file" but type's value is a runtime concern)
        //
        // `bind:value` and `bind:group` are deferred: their type
        // depends on the input's `type` attribute value
        // (`type="number"` → number, default → string, etc.), which
        // requires attribute-aware element-type dispatch. Tracked in
        // NEXT.md as remaining Item 8 scope.
        //
        // Emit direction is assignment (same lambda shape as Item 6):
        // the user's declared target type must accept the element
        // property's type. Wrong-typed targets fire TS2322.
        "checked" => Some("HTMLInputElement['checked']"),
        "files" => Some("HTMLInputElement['files']"),

        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_observer_family() {
        assert_eq!(type_for("contentRect"), Some("DOMRectReadOnly"));
        assert_eq!(type_for("contentBoxSize"), Some("ResizeObserverSize[]"));
        assert_eq!(type_for("borderBoxSize"), Some("ResizeObserverSize[]"));
        assert_eq!(
            type_for("devicePixelContentBoxSize"),
            Some("ResizeObserverSize[]"),
        );
    }

    #[test]
    fn media_time_range_family() {
        assert!(
            type_for("buffered")
                .unwrap()
                .contains("SvelteMediaTimeRange")
        );
        assert!(type_for("played").unwrap().contains("SvelteMediaTimeRange"));
        assert!(
            type_for("seekable")
                .unwrap()
                .contains("SvelteMediaTimeRange")
        );
    }

    #[test]
    fn layout_measurements_use_html_element() {
        // v0.3 Item 6: re-added after moving the emit of the
        // `__svn_any_as<...>(EXPR);` contract call inline at the
        // bind-site, so block-scoped iterator names resolve.
        assert_eq!(type_for("clientWidth"), Some("HTMLElement['clientWidth']"));
        assert_eq!(
            type_for("clientHeight"),
            Some("HTMLElement['clientHeight']")
        );
        assert_eq!(type_for("offsetWidth"), Some("HTMLElement['offsetWidth']"));
        assert_eq!(
            type_for("offsetHeight"),
            Some("HTMLElement['offsetHeight']")
        );
    }

    #[test]
    fn image_dimensions_use_html_image_element() {
        // naturalWidth/Height are HTMLImageElement-specific.
        assert_eq!(
            type_for("naturalWidth"),
            Some("HTMLImageElement['naturalWidth']")
        );
        assert_eq!(
            type_for("naturalHeight"),
            Some("HTMLImageElement['naturalHeight']")
        );
    }

    #[test]
    fn media_props_use_html_media_element() {
        // duration/seeking/ended/readyState live on HTMLMediaElement.
        assert_eq!(type_for("duration"), Some("HTMLMediaElement['duration']"));
        assert_eq!(type_for("seeking"), Some("HTMLMediaElement['seeking']"));
        assert_eq!(type_for("ended"), Some("HTMLMediaElement['ended']"));
        assert_eq!(
            type_for("readyState"),
            Some("HTMLMediaElement['readyState']")
        );
    }

    #[test]
    fn bidirectional_bindings_partial_coverage() {
        // v0.3 Item 8 (narrow): bind:checked and bind:files have
        // fixed HTMLInputElement types, so they ARE in the table now.
        assert_eq!(type_for("checked"), Some("HTMLInputElement['checked']"));
        assert_eq!(type_for("files"), Some("HTMLInputElement['files']"));
        // bind:value is context-aware (tag + `type` attribute); the
        // static `type_for` table returns None. Dispatch happens via
        // `template_walker::resolve_bind_value_type(tag, attrs)`.
        assert_eq!(type_for("value"), None);
        // bind:group is intentionally skipped — upstream widens to any
        // (`__sveltets_2_any(null)`); we mirror by staying silent.
        assert_eq!(type_for("group"), None);
        // `bind:this` is handled via a different path entirely
        // (`collect_bind_this_checks` + inline emit at Item 7).
        assert_eq!(type_for("this"), None);
    }

    #[test]
    fn unknown_names_return_none() {
        assert_eq!(type_for("foo"), None);
        assert_eq!(type_for(""), None);
    }
}
