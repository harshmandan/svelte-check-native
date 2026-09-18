//! `{#each}` block emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/EachBlock.ts`.

use std::collections::HashMap;
use std::fmt::Write;

use svn_parser::EachBlock;

use crate::emit_buffer::EmitBuffer;
use crate::emit_template_body;

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
    // `{#each expr, i}` (index-only sequence form) has an as-clause with
    // no context pattern — the placeholder binding covers it like the
    // clause-less `{#each items}`, and is referenced once so an unused
    // placeholder never reports.
    let context: Option<(&str, svn_core::Range)> = b
        .as_clause
        .as_ref()
        .and_then(|c| c.context_range)
        .and_then(|r| source.get(r.start as usize..r.end as usize).map(|t| (t, r)));
    let binding_text = context.map_or("__svn_each_unused", |(t, _)| t);
    let index: Option<(&str, svn_core::Range)> = b
        .as_clause
        .as_ref()
        .and_then(|c| c.index_range)
        .and_then(|r| source.get(r.start as usize..r.end as usize).map(|t| (t, r)));
    // `{#each true, items as item}` is valid; the sequence needs parens
    // to stay one argument (`EachBlock.ts`'s `containsComma`).
    let (open, close) = if expr_text.contains(',') {
        ("(", ")")
    } else {
        ("", "")
    };
    let write_binding = |buf: &mut EmitBuffer| match context {
        Some((text, range)) => buf.append_with_source(text, range),
        None => buf.push_str(binding_text),
    };
    // `{#each items as items}` names the iterable and the item the same.
    // Emitting `for (let items of __svn_each_items(items))` directly
    // fires TS2448 / TS7022 — the for-of binding shadows the iterable
    // reference. Mirror upstream EachBlock.ts's `arrayAndItemVarTheSame`
    // path: bind the iterable to a temp in a wrapper block first, then
    // iterate the temp. (The temp is block-scoped, so nested same-name
    // each blocks shadow cleanly rather than colliding.)
    let same_name = context.is_some() && !expr_text.is_empty() && binding_text == expr_text;
    if same_name {
        let _ = write!(
            buf,
            "{indent}{{ const __svn_each_arr = __svn_each_items({open}"
        );
    } else {
        let _ = write!(buf, "{indent}for (let ");
        write_binding(buf);
        let _ = write!(buf, " of __svn_each_items({open}");
    }
    match expr_source_range {
        Some(r) => buf.append_with_source(expr_text, r),
        None => buf.push_str(expr_text),
    }
    if same_name {
        let _ = write!(buf, "{close}); for (let ");
        write_binding(buf);
        let _ = writeln!(buf, " of __svn_each_arr) {{");
    } else {
        let _ = writeln!(buf, "{close})) {{");
    }
    if context.is_none() {
        let _ = writeln!(buf, "{indent}    {binding_text};");
    }
    // `let i = 1;` — the index, typed `number` by inference in both
    // TypeScript and JavaScript overlays, as upstream writes it.
    if let Some((text, range)) = index {
        let _ = write!(buf, "{indent}    let ");
        buf.append_with_source(text, range);
        let _ = writeln!(buf, " = 1;");
    }
    // The `(key)` of a keyed each block is user code and gets checked
    // like any other template expression. It belongs inside the loop
    // because it is evaluated per iteration and typically references
    // the item binding. Emitting it also makes anything used only from
    // a key count as used, so `noUnusedLocals` doesn't report a prop
    // that the key is the sole consumer of. Parenthesised so a key that
    // is an object literal stays an expression instead of parsing as a
    // block. Mirrors upstream EachBlock.ts, which appends `;` to the
    // key in place at the same spot.
    if let Some(key_range) = b.as_clause.as_ref().and_then(|c| c.key_range)
        && let Some(key_text) = source.get(key_range.start as usize..key_range.end as usize)
    {
        let _ = write!(buf, "{indent}    (");
        buf.append_with_source(key_text, key_range);
        let _ = writeln!(buf, ");");
    }
    emit_template_body(buf, source, &b.body, depth + 1, insts, action_counter);
    let _ = writeln!(buf, "{indent}}}");
    if same_name {
        // Close the `arrayAndItemVarTheSame` wrapper block.
        let _ = writeln!(buf, "{indent}}}");
    }

    if let Some(alt) = &b.alternate {
        emit_template_body(buf, source, alt, depth, insts, action_counter);
    }
}
