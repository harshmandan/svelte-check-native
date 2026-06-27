//! `{@debug a, b}` debug tag.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/DebugTag.ts`.
//!
//! Upstream emits each comma-separated identifier as a bare statement
//! (`{@debug a, b}` → `;a;b;`) so tsgo type-checks each identifier
//! against the surrounding scope.
//!
//! We split the body on top-level commas and emit one
//! `(IDENT);` per part, with a TokenMapEntry per part so TS2304
//! diagnostics land at the user's source position rather than the
//! `{@debug` keyword.

use crate::emit_buffer::EmitBuffer;

/// Emit `{@debug a, b, …}` as one bare `(IDENT);` per comma-separated
/// part so tsgo fires TS2304 on typo'd names.
///
/// Top-level commas only — `{@debug a, b, c}` splits into three parts.
/// Depth tracking matches `()`, `[]`, `{}`, `<>` so any commas nested in
/// a (parse-error-only) non-identifier body wouldn't split. Svelte's
/// parser restricts `{@debug}` to a comma-separated identifier list, so
/// in practice every part is a bare name.
pub(crate) fn emit_debug_tag(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
) {
    let expr_start = interp.expression_range.start as usize;
    let expr_end = interp.expression_range.end as usize;
    let Some(body_raw) = source.get(expr_start..expr_end) else {
        return;
    };
    if body_raw.trim().is_empty() {
        // `{@debug}` with no expressions — runtime "log every reactive
        // value" form. Nothing to type-check.
        return;
    }
    let indent = "    ".repeat(depth);
    let body_start_offset = interp.expression_range.start;
    for (rel_start, rel_end) in split_top_level_commas(body_raw) {
        let part = &body_raw[rel_start..rel_end];
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }
        let leading_ws = (part.len() - part.trim_start().len()) as u32;
        let abs_start = body_start_offset + rel_start as u32 + leading_ws;
        let abs_end = abs_start + trimmed.len() as u32;
        buf.append_synthetic(&indent);
        buf.append_synthetic("(");
        buf.append_with_source(trimmed, svn_core::Range::new(abs_start, abs_end));
        buf.append_synthetic(");\n");
    }
}

/// Split `body` on top-level commas, returning `(start, end)` byte
/// offsets for each part. Depth tracking over `() [] {}` keeps
/// call/index/object-literal commas from splitting a body our lenient
/// parser tolerates; Svelte's own `{@debug}` only permits bare
/// identifiers, so this is defensive coverage. `<>` is included
/// belt-and-braces and is not a correct bracket pair in general JS.
fn split_top_level_commas(body: &str) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let bytes = body.as_bytes();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' if depth > 0 => depth -= 1,
            b',' if depth == 0 => {
                out.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push((start, bytes.len()));
    out
}
