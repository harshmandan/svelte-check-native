//! `{expr}` mustache-tag emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/MustacheTag.ts`.
//!
//! `{@const}` is dispatched separately — see [`crate::nodes::const_tag`].
//! Other `{@…}` tags (`@html`, `@render`, `@debug`, `@attach`) are
//! side-effect-only at the TS overlay layer and produce no structured
//! output today.

use crate::emit_buffer::EmitBuffer;
use crate::nodes::const_tag::emit_at_const_if_any;

/// Emit a `{expr}` interpolation as a bare-paren-call expression
/// statement (`(EXPR);`) in the current scope so tsgo type-checks EXPR
/// against any enclosing control-flow narrowing (`{#if}` / `{:else if}` /
/// `{#each …}`) and against the script-declared types of referenced
/// identifiers.
///
/// Routes `{@const}` / `{@html}` / `{@render}` / `{@debug}` through
/// `emit_at_const_if_any`; only `@const` produces structured output today.
pub(crate) fn emit_interpolation(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
) {
    if interp.kind != svn_parser::InterpolationKind::Expression {
        emit_at_const_if_any(buf, source, interp, depth);
        return;
    }
    let expr_start = interp.expression_range.start as usize;
    let expr_end = interp.expression_range.end as usize;
    let Some(expr_raw) = source.get(expr_start..expr_end) else {
        return;
    };
    let trimmed = expr_raw.trim();
    if trimmed.is_empty() {
        return;
    }
    // Shift the source range to point at the trimmed expression so the
    // TokenMapEntry's overlay span exactly matches the source bytes
    // tsgo would blame for a diagnostic.
    let leading_ws = expr_raw.len() - expr_raw.trim_start().len();
    let trimmed_source_start = interp.expression_range.start + leading_ws as u32;
    let trimmed_source_end = trimmed_source_start + trimmed.len() as u32;
    let indent = "    ".repeat(depth);
    buf.append_synthetic(&indent);
    buf.append_synthetic("(");
    buf.append_with_source(
        trimmed,
        svn_core::Range::new(trimmed_source_start, trimmed_source_end),
    );
    buf.append_synthetic(");\n");
}
