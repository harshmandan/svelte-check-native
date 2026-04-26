//! Byte-range / source-text manipulation helpers used by the per-
//! node-type emitters.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/utils/node-utils.ts`.
//!
//! **Status: most upstream helpers are NA by architecture.** Our
//! emit is structural (we build a new overlay from scratch via
//! `EmitBuffer::push_str` / `append_with_source` etc.) rather than
//! upstream's text-rewrite approach (overwrite ranges of the user's
//! source via MagicString). Helpers that mutate byte ranges in-place
//! aren't applicable.
//!
//! ## Per-helper mapping
//!
//! | Upstream helper | Our equivalent |
//! |---|---|
//! | `surroundWith(str, [start, end], prefix, suffix)` | NA — wraps a byte range with prefix/suffix via MagicString. Our emit writes bytes in order; surrounding is handled inline at each emit site. |
//! | `getDirectiveNameStartEndIdx(source, idx)` | inline arithmetic in `nodes::action::emit_dom_action_decls`, `nodes::animation::emit_animation_directive`, `nodes::transition::emit_transition_directive`, `nodes::binding::emit_element_bind_checks_inline` — `d.range.start + d.kind.as_str().len() + 1` (the `+1` for the `:` prefix separator). Could be lifted into a `pub(crate) fn directive_name_range(d: &Directive) -> Range` helper if a 5th call site appears. |
//! | `withTrailingPropertyAccess(source, idx)` | NA — lookahead for trailing `?.foo` after an expression range. Our parser captures full expression spans up front via oxc, so we don't need to extend ranges post-hoc. |
//! | `rangeWithTrailingPropertyAccess(source, range)` | NA — same. |
//! | `sanitizePropName(name)` | partial — `crate::util::is_simple_js_identifier` covers the "is this a valid JS identifier?" question. Upstream's `sanitizePropName` additionally rewrites unsafe names to quoted strings; we handle the rewrite at each prop-emit site (e.g. `nodes::inline_component::write_object_key`). |
//! | `transform(source, transformations: TransformationArray)` | NA — applies a list of MagicString operations in one pass. Our emit doesn't have a transformation-list step; we write directly into the buffer in order. |
//! | `TransformationArray` (type alias) | NA — type alias for the bulk-mutation list shape. Not applicable. |
//!
//! ## Should we fold these into a real Rust module?
//!
//! Probably not. The two helpers that map cleanly to our architecture
//! (`getDirectiveNameStartEndIdx` and `sanitizePropName`) are 1-3 line
//! inlines that would lose context if extracted. The rest are
//! architecturally NA. This file is a navigational stub only.
