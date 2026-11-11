//! `{@attach EXPR}` attachment emission (Svelte 5.29+).
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/AttachTag.ts`:
//! every attachment becomes a computed-symbol property,
//! `[Symbol("@attach")]: EXPR`, in the props literal of the element or
//! component it sits on.
//!
//! The symbol key is what types the attachment. An element's
//! `HTMLAttributes` carry `[key: symbol]: Attachment<T> | …`, and a
//! component that forwards attachments declares a symbol index signature
//! in its props, so the arrow's parameter gets the element type from the
//! object it is written into. Written as a spread (`...EXPR`) the arrow
//! has no contextual type and its parameter is implicitly `any`.

use svn_core::Range;
use svn_parser::AttachTag;

use crate::emit_buffer::EmitBuffer;

/// The attachment expression without surrounding whitespace (the span of
/// the expression node svelte2tsx copies), or `None` when it is empty.
pub(crate) fn expression_range(source: &str, a: &AttachTag) -> Option<Range> {
    let text = a.expression_range.slice(source);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let start = a.expression_range.start + (text.len() - text.trim_start().len()) as u32;
    Some(Range::new(start, start + trimmed.len() as u32))
}

/// Write `[Symbol("@attach")]: EXPR` (no separator) for an expression
/// range from [`expression_range`].
pub(crate) fn emit_attach_entry(buf: &mut EmitBuffer, source: &str, expr: Range) {
    buf.push_str("[Symbol(\"@attach\")]: ");
    buf.append_with_source(expr.slice(source), expr);
}
