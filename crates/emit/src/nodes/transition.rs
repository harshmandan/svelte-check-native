//! `transition:NAME(PARAMS)` / `in:NAME(PARAMS)` / `out:NAME(PARAMS)`
//! directives.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Transition.ts`.
//!
//! Upstream emits a typed call wrapped in
//! `__sveltets_2_ensureTransition(...)`:
//!
//! ```text
//!     __sveltets_2_ensureTransition(
//!         NAME(svelte.mapElementTag('tag'), (PARAMS))
//!     );
//! ```
//!
//! All three directive variants (`transition:` / `in:` / `out:`) share
//! the same call shape and route to the same handler.
//!
//! We emit the bare call (no `__svn_ensure_transition` wrapper yet —
//! same trade-off as `animation.rs`). The bare emit covers TS2304 /
//! TS2554 / TS2345 by letting tsgo type-check NAME's signature
//! against the call.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// Emit `transition:NAME` / `in:NAME` / `out:NAME` (with or without
/// `={PARAMS}`) as a typed call so tsgo type-checks NAME's signature.
///
/// The directive name is emitted via `append_with_source` so a TS2304
/// on a typo'd transition name maps back to the source position via
/// the token map.
pub(crate) fn emit_transition_directive(
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
    // Compute the source range covering the directive NAME (after
    // `transition:` / `in:` / `out:`).
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
            let _ = write!(buf, "(__svn_map_element_tag({tag_arg}), (");
            buf.append_with_source(params, *expression_range);
            buf.push_str(")));\n");
        }
        _ => {
            // Bare `transition:fade` (no params expression). Params
            // slot is optional in Svelte's transition signature.
            buf.push_str(indent);
            buf.push_str("(");
            buf.append_with_source(name, name_range);
            let _ = writeln!(buf, "(__svn_map_element_tag({tag_arg})));");
        }
    }
}
