//! `{#key EXPR}` block emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Key.ts`.
//!
//! `{#key EXPR}body{/key}` re-creates the body when EXPR changes. From
//! the type-checker's perspective the body has the same scope as the
//! enclosing template — there's no introduced binding to check
//! against — but EXPR itself is a real script-side expression and must
//! be type-checked so a typo / out-of-scope reference fires
//! TS2304 / TS2552 just like any `{expr}` interpolation. We emit it
//! as a bare `;(EXPR);` statement preceding the body walk, matching
//! the shape `mustache_tag::emit_plain_expression` uses for plain
//! `{expr}`.

use std::collections::HashMap;

use crate::emit_buffer::EmitBuffer;

/// Emit a `{#key EXPR}body{/key}` block as `;(EXPR);` (so tsgo
/// type-checks the expression) followed by the body walk.
///
/// Leading `;` guards against an immediately-preceding statement that
/// looks like a function-call lhs (`x\n(EXPR)` would be parsed as
/// `x(EXPR)`). The wrapping parens guard against object-literal-vs-
/// block ambiguity at expression-statement position. Trailing `;\n`
/// closes the statement.
pub(crate) fn emit_key_block(
    buf: &mut EmitBuffer,
    source: &str,
    b: &svn_parser::KeyBlock,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let expr_start = b.expression_range.start as usize;
    let expr_end = b.expression_range.end as usize;
    if let Some(expr_raw) = source.get(expr_start..expr_end) {
        let trimmed = expr_raw.trim();
        if !trimmed.is_empty() {
            // Shift the source range to point at the trimmed expression
            // so a TS2304/TS2552 diagnostic blames the exact identifier
            // bytes rather than including leading whitespace.
            let leading_ws = expr_raw.len() - expr_raw.trim_start().len();
            let trimmed_source_start = b.expression_range.start + leading_ws as u32;
            let trimmed_source_end = trimmed_source_start + trimmed.len() as u32;
            let indent = "    ".repeat(depth);
            buf.append_synthetic(&indent);
            buf.append_synthetic(";(");
            buf.append_with_source(
                trimmed,
                svn_core::Range::new(trimmed_source_start, trimmed_source_end),
            );
            buf.append_synthetic(");\n");
        }
    }
    crate::emit_template_body(buf, source, &b.body, depth, insts, action_counter);
}
