//! `{@attach EXPR}` attachment-directive emission (Svelte 5.29+).
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/AttachTag.ts`.
//!
//! Emit as a computed-symbol property (`[Symbol("@attach")]: EXPR`) so
//! the arrow's parameter picks up the element's contextual type via
//! the `[key: symbol]: Attachment<T> | …` index signature on
//! `HTMLAttributes` in svelte/elements.d.ts.
//!
//! A `...EXPR` spread emit (the pre-2026-04-25 shape) would leave the
//! arrow's param as implicit-any — tsgo then fires TS7006 on common
//! patterns like `{@attach el => el.focus()}`. The computed-symbol
//! shape is what flows the contextual type into the callback.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// Emit one `{@attach EXPR}` entry into an open `createElement` props
/// literal. `inner` is the per-entry indentation prefix (caller has
/// already pre-computed it from the element's `depth`).
pub(crate) fn emit_attach(
    buf: &mut EmitBuffer,
    expr_text: &str,
    source_range: svn_core::Range,
    inner: &str,
) {
    let _ = write!(buf, "{inner}[Symbol(\"@attach\")]: ");
    buf.append_with_source(expr_text, source_range);
    let _ = writeln!(buf, ",");
}
