//! `{@render foo(x)}` snippet-render tag (Svelte 5+).
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/RenderTag.ts`,
//! which emits `;__sveltets_2_ensureSnippet(foo(x));`. The wrapper's
//! parameter is Svelte's branded snippet-return type, so tsgo checks
//! both the call itself (missing names, arity, argument types) and that
//! the callee really is a snippet: rendering a plain function that
//! returns `void` or `string` fails the wrapper's argument check.

use crate::emit_buffer::EmitBuffer;

/// Emit `{@render EXPR}` as `__svn_ensure_snippet(EXPR);`.
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
    buf.append_synthetic("__svn_ensure_snippet(");
    buf.append_with_source(
        trimmed,
        svn_core::Range::new(trimmed_source_start, trimmed_source_end),
    );
    buf.append_synthetic(");\n");
}
