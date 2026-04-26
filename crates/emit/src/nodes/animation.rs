//! `animate:NAME(PARAMS)` animation directive.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Animation.ts`.
//!
//! Upstream emits a typed call wrapped in
//! `__sveltets_2_ensureAnimation(...)`:
//!
//! ```text
//!     __sveltets_2_ensureAnimation(
//!         NAME(svelte.mapElementTag('tag'), __sveltets_2_AnimationMove, (PARAMS))
//!     );
//! ```
//!
//! We emit the bare call (no `__svn_ensure_animation` wrapper yet —
//! adding it would constrain the return type to `AnimationConfig`,
//! catching "you returned a function instead of an animation config"
//! errors. For the first cut we cover TS2304 / TS2554 / TS2345 by
//! emitting the call directly; the `ensure` wrapper is a future
//! refinement).

use std::fmt::Write;

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
    match &d.value {
        Some(svn_parser::DirectiveValue::Expression {
            expression_range, ..
        }) => {
            let Some(params) =
                source.get(expression_range.start as usize..expression_range.end as usize)
            else {
                return;
            };
            buf.push_str(indent);
            buf.push_str("(");
            buf.append_with_source(name, name_range);
            let _ = write!(
                buf,
                "(__svn_map_element_tag({tag_arg}), __svn_AnimationMove, ("
            );
            buf.append_with_source(params, *expression_range);
            buf.push_str(")));\n");
        }
        _ => {
            // Bare `animate:flip` (no params expression). Emit
            // without the third arg; tsgo accepts it because the
            // params slot is declared optional in Svelte's animation
            // signature.
            buf.push_str(indent);
            buf.push_str("(");
            buf.append_with_source(name, name_range);
            let _ = writeln!(
                buf,
                "(__svn_map_element_tag({tag_arg}), __svn_AnimationMove));"
            );
        }
    }
}
