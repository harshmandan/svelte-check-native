//! `{expr}` mustache-tag emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/MustacheTag.ts`.
//!
//! Dispatches to per-tag handlers for `{@const}`, `{@html}`,
//! `{@render}`, `{@debug}` based on the parser's
//! [`svn_parser::InterpolationKind`] discriminator. `@attach` lives on
//! the element-spread path (see [`crate::nodes::attach_tag`]) and
//! doesn't reach this dispatcher.

use crate::emit_buffer::EmitBuffer;
use crate::nodes::const_tag::{emit_at_const_if_any, emit_declaration_tag};
use crate::nodes::debug_tag::emit_debug_tag;
use crate::nodes::raw_mustache_tag::emit_raw_html;
use crate::nodes::render_tag::emit_render_tag;

/// Emit a `{expr}` interpolation as an expression statement (`EXPR;`)
/// in the current scope so tsgo type-checks EXPR
/// against any enclosing control-flow narrowing (`{#if}` / `{:else if}` /
/// `{#each …}`) and against the script-declared types of referenced
/// identifiers.
///
/// `{@const}` / `{@html}` / `{@render}` / `{@debug}` route to their
/// dedicated handlers. Other `{@*}` tags (currently only the catch-all
/// `AtTag` for forward-compat) emit no overlay output.
pub(crate) fn emit_interpolation(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
) {
    use svn_parser::InterpolationKind::*;
    match interp.kind {
        Expression => emit_plain_expression(buf, source, interp, depth),
        AtConst => emit_at_const_if_any(buf, source, interp, depth),
        DeclConst | DeclLet => emit_declaration_tag(buf, source, interp, depth),
        AtHtml => emit_raw_html(buf, source, interp, depth),
        AtRender => emit_render_tag(buf, source, interp, depth),
        AtDebug => emit_debug_tag(buf, source, interp, depth),
        AtTag => {} // forward-compat catch-all; no emit
    }
}

/// `{EXPR}` plain interpolation, emitted the way svelte2tsx's
/// `MustacheTag.ts` rewrites it: the braces become `EXPR;`, or
/// `;(EXPR);` when the expression opens with `{` (an object literal
/// that would otherwise read as a block). The text matters beyond its
/// meaning: when the overlay around it is already syntactically broken
/// (a script block svelte2tsx did not recognise is copied into the
/// template verbatim), the TypeScript parser's recovery errors depend
/// on the exact tokens, so they have to be svelte2tsx's. The
/// replacement punctuation maps back to the brace it replaces, as
/// svelte2tsx's content-only overwrite of that brace does.
fn emit_plain_expression(
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
    // Shift the source range to point at the trimmed expression so the
    // TokenMapEntry's overlay span exactly matches the source bytes
    // tsgo would blame for a diagnostic.
    let leading_ws = expr_raw.len() - expr_raw.trim_start().len();
    let trimmed_source_start = interp.expression_range.start + leading_ws as u32;
    let trimmed_source_end = trimmed_source_start + trimmed.len() as u32;
    let open_brace = svn_core::Range::new(interp.range.start, interp.range.start + 1);
    // svelte2tsx keeps the whitespace inside the braces and overwrites
    // only the `}`, so the first character after the expression — that
    // whitespace, or the brace itself — is where a range ending with the
    // expression lands. The trimmed emit's closing punctuation stands
    // for that character.
    let after_expr = svn_core::Range::new(trimmed_source_end, trimmed_source_end + 1);
    let object_like = trimmed.starts_with('{');
    let indent = "    ".repeat(depth);
    buf.append_synthetic(&indent);
    if object_like {
        buf.append_with_source(";(", open_brace);
    }
    buf.append_with_source(
        trimmed,
        svn_core::Range::new(trimmed_source_start, trimmed_source_end),
    );
    buf.append_with_source(if object_like { ");" } else { ";" }, after_expr);
    buf.append_synthetic("\n");
}
