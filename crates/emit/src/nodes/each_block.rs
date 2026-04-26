//! `{#each}` block emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/EachBlock.ts`.

use std::collections::HashMap;
use std::fmt::Write;

use svn_parser::EachBlock;

use crate::emit_buffer::EmitBuffer;
use crate::{all_identifiers, emit_is_ts, emit_template_body};

/// Emit a `for`-of loop for an `{#each}` block.
///
/// `{#each items}` without an `as` clause is legal Svelte (iterate N times,
/// discard the value); we use `__svn_each_unused` as a placeholder binding
/// so the emitted TypeScript stays syntactically valid.
pub(crate) fn emit_each_block(
    buf: &mut EmitBuffer,
    source: &str,
    b: &EachBlock,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let indent = "    ".repeat(depth);
    let raw_expr = source
        .get(b.expression_range.start as usize..b.expression_range.end as usize)
        .unwrap_or("undefined");
    let expr_text = raw_expr.trim();
    // Trimmed-slice source range so tsgo diagnostics fired anywhere
    // inside the expression (e.g. TS18048 on `.sort((a, b) => …)`
    // callback params) map back to their actual user-source byte
    // positions via the token map. Without this, callback-param
    // diagnostics fall in a synthesized region (the `__svn_each_items(...)`
    // wrapper span) and `map_diagnostic` drops them.
    let expr_source_range: Option<svn_core::Range> = if expr_text.is_empty() {
        None
    } else {
        let leading_ws = (raw_expr.len() - raw_expr.trim_start().len()) as u32;
        let start = b.expression_range.start + leading_ws;
        Some(svn_core::Range::new(start, start + expr_text.len() as u32))
    };
    let binding_text = match &b.as_clause {
        Some(c) => source
            .get(c.context_range.start as usize..c.context_range.end as usize)
            .unwrap_or("__svn_each_unused")
            .to_string(),
        None => "__svn_each_unused".to_string(),
    };
    // `{#each items as item, i}` — `i` is the zero-based iteration index.
    // `__svn_each_items` returns a plain Iterable, which doesn't expose
    // `.entries()`, so declaring the index as a `const i: number` inside
    // the loop body is simpler than rewriting the for-of pattern to
    // destructure an [index, item] pair. Using `0` for the value is a
    // type-only trick — the generated function is never executed.
    let index_binding: Option<&str> = b
        .as_clause
        .as_ref()
        .and_then(|c| c.index_range)
        .and_then(|r| source.get(r.start as usize..r.end as usize));
    let _ = write!(
        buf,
        "{indent}for (const {binding_text} of __svn_each_items("
    );
    match expr_source_range {
        Some(r) => buf.append_with_source(expr_text, r),
        None => buf.push_str(expr_text),
    }
    let _ = writeln!(buf, ")) {{");
    if let Some(ix) = index_binding {
        if emit_is_ts() {
            let _ = writeln!(buf, "{indent}    const {ix}: number = 0;");
        } else {
            let _ = writeln!(buf, "{indent}    /** @type {{number}} */ const {ix} = 0;");
        }
    }
    emit_template_body(buf, source, &b.body, depth + 1, insts, action_counter);
    // Void every identifier that the binding pattern destructures, not
    // just the first. `[id, label]` and `[id, { label }]` both bind two
    // names and TS6133 fires on each unused one.
    for ident in all_identifiers(&binding_text) {
        let _ = writeln!(buf, "{indent}    void {ident};");
    }
    if let Some(ix) = index_binding {
        let _ = writeln!(buf, "{indent}    void {ix};");
    }
    let _ = writeln!(buf, "{indent}}}");

    if let Some(alt) = &b.alternate {
        emit_template_body(buf, source, alt, depth, insts, action_counter);
    }
}
