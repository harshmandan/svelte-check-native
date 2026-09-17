//! How a JS component's `$props()` gets its type.
//!
//! Upstream svelte2tsx (`ExportedNames.handle$propsRune`) reads a JS
//! component's props type from the `@type` comment leading the
//! `$props()` declaration ([`PropsInfo::props_type_comment`]). Only when
//! there is none does it synthesise `$$ComponentProps` from the
//! destructure. Other JSDoc in the script — `@typedef` blocks included —
//! plays no part.

use crate::PropsInfo;

/// Whether a JS component's props come from a `$$ComponentProps`
/// typedef synthesised from its `$props()` destructure: there is a
/// destructure to read and no `@type` comment typing it.
pub fn should_synthesise_js_props(props_info: &PropsInfo) -> bool {
    props_info.props_type_comment.is_none()
        && (props_info.first_props_call_len > 0 || props_info.props_with_unknown)
}
