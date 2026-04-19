//! Per-DOM-binding type table for one-way-not-on-element `bind:`
//! directives.
//!
//! The ResizeObserver / HTMLMediaElement binding family carries types
//! that live on SEPARATE browser APIs rather than on the bound
//! element itself. `<div bind:contentRect={rect}>` for instance
//! doesn't assign `div.contentRect` — there's no such property on
//! `HTMLDivElement`. Svelte wires up a ResizeObserver behind the
//! scenes and delivers `DOMRectReadOnly` into `rect`.
//!
//! For type-checking we don't need to model the runtime plumbing —
//! just provide the TYPE so the assignment `rect = <RHS>` checks
//! `rect`'s declared type accepts it. Upstream svelte2tsx does the
//! same via its `oneWayBindingAttributesNotOnElement` map.
//!
//! Bidirectional bindings (`bind:value`, `bind:checked`, etc.) are
//! intentionally NOT here — those need element-specific type
//! resolution and belong in a broader element-type table (future
//! work).

/// Return the TS type to assert for `bind:NAME` when the binding is
/// one-way AND the property doesn't live on the element. `None`
/// means either the binding is element-native (handled by a
/// different path) or not one we model.
///
/// Names taken verbatim from upstream svelte2tsx's
/// `oneWayBindingAttributesNotOnElement` map so parity with
/// upstream's type-check behavior is preserved.
pub fn type_for(binding_name: &str) -> Option<&'static str> {
    match binding_name {
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
        assert!(type_for("buffered").unwrap().contains("SvelteMediaTimeRange"));
        assert!(type_for("played").unwrap().contains("SvelteMediaTimeRange"));
        assert!(type_for("seekable").unwrap().contains("SvelteMediaTimeRange"));
    }

    #[test]
    fn element_native_bindings_return_none() {
        // These live on the element directly (via HTMLInputElement etc.)
        // and are handled via a different path — type_for returns None.
        assert_eq!(type_for("value"), None);
        assert_eq!(type_for("checked"), None);
        assert_eq!(type_for("clientWidth"), None);
        assert_eq!(type_for("this"), None);
    }

    #[test]
    fn unknown_names_return_none() {
        assert_eq!(type_for("foo"), None);
        assert_eq!(type_for(""), None);
    }
}
