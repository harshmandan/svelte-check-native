//! `animate:NAME(PARAMS)` animation directive.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Animation.ts`,
//! which emits the typed call wrapped in `__sveltets_2_ensureAnimation(...)`:
//!
//! ```text
//!     __sveltets_2_ensureAnimation(
//!         NAME(svelte.mapElementTag('tag'), __sveltets_2_AnimationMove, (PARAMS))
//!     );
//! ```
//!
//! The call checks NAME's signature (missing name, arity, parameter
//! types); the wrapper checks its result is an animation config, so an
//! animation function that returns something else (a cleanup thunk, a
//! number) is rejected.

use crate::emit_buffer::EmitBuffer;

/// Emit `animate:NAME` / `animate:NAME(PARAMS)` as a typed call so
/// tsgo type-checks NAME's signature against the call shape.
///
/// The directive name is emitted via `append_with_source` so a TS2304
/// "Cannot find name 'NAME'" diagnostic on a typo'd animation name
/// maps back to the user's `animate:NAME` source position via the
/// token map. Without this, the diagnostic lands inside synthesized
/// scaffolding (no line_map coverage) and the diagnostic mapper drops
/// it.
pub(crate) fn emit_animation_directive(
    buf: &mut EmitBuffer,
    source: &str,
    d: &svn_parser::Directive,
    indent: &str,
    tag_name: &str,
) {
    let name = d.name.as_str();
    let tag_arg = if tag_name.is_empty() {
        "'' as string".to_string()
    } else {
        format!("'{tag_name}'")
    };
    // Compute the source range covering the directive NAME — used
    // below to pin TS2304 diagnostics to the `animate:flip` site.
    // `d.range.start` is the byte offset of the `animate:` prefix; the
    // name starts after `animate:` (kind str + 1 for the colon).
    let prefix_len = (d.kind.as_str().len() + 1) as u32;
    let name_start = d.range.start + prefix_len;
    let name_end = name_start + name.len() as u32;
    let name_range = svn_core::Range::new(name_start, name_end);
    // The element and move arguments are ours, not the user's. A
    // diagnostic on one of them (an animation declaring fewer
    // parameters, or a narrower element type) lands on the last
    // character of the directive name, which is where upstream's
    // source map resolves text inserted right after the name.
    let synthetic_args = format!("(__svn_map_element_tag({tag_arg}), __svn_AnimationMove");
    let name_tail = svn_core::Range::new(name_end.saturating_sub(1), name_end);
    buf.push_str(indent);
    buf.push_str("__svn_ensure_animation(");
    buf.append_with_source(name, name_range);
    buf.append_with_source(&synthetic_args, name_tail);
    match &d.value {
        Some(svn_parser::DirectiveValue::Expression {
            expression_range, ..
        }) => {
            let params = source
                .get(expression_range.start as usize..expression_range.end as usize)
                .unwrap_or("");
            buf.push_str(", (");
            buf.append_with_source(params, *expression_range);
            buf.push_str(")));\n");
        }
        // Bare `animate:flip`: the params slot is optional in Svelte's
        // animation signature, so the call omits it.
        _ => buf.push_str("));\n"),
    }
}
