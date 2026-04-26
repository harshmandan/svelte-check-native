//! `{...EXPR}` spread emission inside a `createElement` props literal.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Spread.ts`.
//!
//! `{@attach EXPR}` (Svelte 5.29+) shares the parser's spread shape
//! (`Attribute::Spread { is_attach: true, … }`) but takes a different
//! emit path — see [`crate::nodes::attach_tag`].

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;
use crate::nodes::attach_tag::emit_attach;

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

/// Emit one spread / attach attribute entry into the open
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
    if s.is_attach {
        emit_attach(buf, trimmed, svn_core::Range::new(start, end), &inner);
    } else {
        let _ = write!(buf, "{inner}...");
        buf.append_with_source(trimmed, svn_core::Range::new(start, end));
        let _ = writeln!(buf, ",");
    }
}
