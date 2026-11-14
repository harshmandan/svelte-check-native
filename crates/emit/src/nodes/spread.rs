//! `{...EXPR}` spread emission inside a `createElement` props literal.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Spread.ts`.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// Quick check — true when the spread has a non-empty trimmed
/// expression that's worth emitting. Used by the element dispatcher
/// to decide whether to flush the leading newline before the first
/// attribute entry.
pub(crate) fn can_emit(source: &str, s: &svn_parser::SpreadAttr) -> bool {
    source
        .get(s.expression_range.start as usize..s.expression_range.end as usize)
        .map(|expr| !expr.trim().is_empty())
        .unwrap_or(false)
}

/// Emit one spread attribute entry into the open
/// `createElement` props literal. Caller must have already checked
/// [`can_emit`] and flushed any pending leading newline.
pub(crate) fn emit_spread(
    buf: &mut EmitBuffer,
    source: &str,
    s: &svn_parser::SpreadAttr,
    depth: usize,
) {
    let Some(expr) = source.get(s.expression_range.start as usize..s.expression_range.end as usize)
    else {
        return;
    };
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return;
    }
    let inner = "    ".repeat(depth + 1);
    let leading_ws = (expr.len() - expr.trim_start().len()) as u32;
    let start = s.expression_range.start + leading_ws;
    let end = start + trimmed.len() as u32;
    buf.push_str(&inner);
    write_spread_dots(buf, s.range);
    buf.append_with_source(trimmed, svn_core::Range::new(start, end));
    let _ = writeln!(buf, ",");
}

/// Writes the `...` of a spread whose source is `range` (`{...x}`),
/// mapped to the source text after the opening brace: svelte2tsx writes
/// a spread as that text, so a position on the `...` maps there.
pub(crate) fn write_spread_dots(buf: &mut EmitBuffer, range: svn_core::Range) {
    let start = range.start + 1;
    buf.append_with_source("...", svn_core::Range::new(start, start + 3));
}
