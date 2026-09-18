//! Emission of hoisted-import statements at module scope.
//!
//! Pulled out of `lib.rs` to keep that file focused on the
//! orchestrator + AST analysis. The hoisting itself is decided in
//! [`crate::process_instance_script_content`]; this module is the
//! emit-time consumer that writes those statements into the overlay
//! and threads source-map metadata so a TS diagnostic on a hoisted
//! import line points at the original `<script>` line.

use crate::LineMapEntry;
use crate::emit_buffer::EmitBuffer;
use crate::process_instance_script_content;
use crate::util::{count_lines, current_line, source_line_at};

/// Emit the hoisted-imports region at module scope, followed by a
/// per-statement LineMapEntry so a diagnostic on a hoisted import line
/// points at the original `<script>` import line. Each hoisted
/// statement was concatenated verbatim into `s.hoisted` — line counts
/// inside a statement match the source 1:1, so we emit one entry per
/// statement.
pub(crate) fn emit_hoisted_imports(
    buf: &mut EmitBuffer,
    split: Option<&process_instance_script_content::SplitScript>,
    doc: &svn_parser::Document<'_>,
) {
    let Some(s) = split else { return };
    if s.hoisted.is_empty() {
        return;
    }
    if let Some(instance) = &doc.instance_script {
        let mut overlay_cursor = current_line(buf.as_str());
        // Each statement's extent inside `hoisted` was recorded when it
        // was concatenated, so the walk is exact. Re-deriving the
        // boundaries from the text used to require a next-statement
        // heuristic ("alpha at column 0"), which never matched hoisted
        // statements that keep their source indentation — the whole
        // region collapsed into one entry anchored at the first
        // import, and a blank source line between imports shifted
        // every diagnostic after it by one.
        for (&source_offset, &(stmt_start, stmt_end)) in
            s.hoisted_byte_offsets.iter().zip(&s.hoisted_stmt_spans)
        {
            let stmt_text = &s.hoisted[stmt_start..stmt_end];
            let stmt_line_count = count_lines(stmt_text).max(1);
            let source_line =
                source_line_at(doc.source, instance.content_range.start + source_offset);
            buf.push_line_map(LineMapEntry {
                overlay_start_line: overlay_cursor,
                overlay_end_line: overlay_cursor + stmt_line_count,
                source_start_line: source_line,
            });
            overlay_cursor += stmt_line_count;
        }
    }
    buf.push_str(&s.hoisted);
    if !s.hoisted.ends_with('\n') {
        buf.push_str("\n");
    }
}
