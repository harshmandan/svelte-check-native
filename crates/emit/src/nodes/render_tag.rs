//! `{@render foo(x)}` snippet-render tag (Svelte 5+).
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/RenderTag.ts`.
//!
//! Upstream emits `;__sveltets_2_ensureSnippet(foo(x));` so tsgo:
//!   1. Type-checks the call's arguments against `foo`'s declared
//!      `Snippet<[…]>` parameter shape.
//!   2. Validates that `foo` IS a snippet (the
//!      `__sveltets_2_ensureSnippet` ambient constrains its arg to
//!      `Snippet<…> | undefined`).
//!
//! We emit `(EXPR);` directly (no `__svn_ensure_snippet` ambient yet —
//! the ambient adds an extra type-narrowing constraint we don't
//! declare). The bare expression statement still type-checks the call
//! itself: TS2304 on missing identifiers, TS2554 on argument arity
//! mismatch, TS2345 on argument type mismatch — covers the common
//! gap-classes. Adding a `__svn_ensure_snippet<T extends Snippet<…>
//! | undefined>(value: T): void` ambient is a future refinement that
//! tightens the "is it actually a snippet?" check.

use crate::emit_buffer::EmitBuffer;

/// Emit `{@render EXPR}` as a bare expression statement so tsgo
/// type-checks the snippet call's arguments against the declared
/// `Snippet<[…]>` parameter shape.
pub(crate) fn emit_render_tag(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
) {
    let expr_start = interp.expression_range.start as usize;
    let expr_end = interp.expression_range.end as usize;
    let Some(expr_raw) = source.get(expr_start..expr_end) else {
        return;
    };
    let trimmed = expr_raw.trim();
    if trimmed.is_empty() {
        return;
    }
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
