//! `{@html EXPR}` raw-HTML tag.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/RawMustacheTag.ts`.
//!
//! Upstream emits the expression as a bare statement (`{@html foo}`
//! becomes `;foo;`) so tsgo type-checks `foo` against the surrounding
//! scope. We emit `(EXPR);` (paren-wrapped to protect against
//! sequence-expression / assignment-looking shapes).

use crate::emit_buffer::EmitBuffer;

/// Emit `{@html EXPR}` as a bare expression statement so tsgo
/// type-checks EXPR against scope (catches TS2304 on typos, TS2339
/// on missing-property reads, etc.).
pub(crate) fn emit_raw_html(
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
