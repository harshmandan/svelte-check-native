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
///
/// `s.hoisted` starts with synthetic `declare const <name>: …;` stubs
/// for body-referenced names — those have NO entry in
/// `hoisted_byte_offsets`, so we skip past them before aligning the
/// walk cursor with the first real offset. Without this skip, every
/// overlay line in the hoist region gets mapped to the wrong source
/// line (every entry's source_offset is applied N-stubs too early).
pub(crate) fn emit_hoisted_imports(
    buf: &mut EmitBuffer,
    split: Option<&process_instance_script_content::SplitScript>,
    doc: &svn_parser::Document<'_>,
    is_ts: bool,
) {
    let Some(s) = split else { return };
    if s.hoisted.is_empty() {
        return;
    }
    // For JS overlays, drop the synthetic `declare const X: { [key:
    // string]: any } & ((...args: any[]) => any);` stub prelude —
    // `declare const` is TS-only syntax that tsgo refuses to parse in
    // a `.js` file (TS8009/TS8010), and even one such error aborts
    // the whole-program type-check (Types: 340 / Instantiations: 0
    // signal). The stubs exist to keep `typeof X` usable in hoisted
    // Props types, which JS overlays don't carry anyway (Props is
    // expressed via JSDoc @typedef, not a real type alias).
    //
    // TS overlays keep the stubs; JS overlays slice them off. Line
    // map entries cover ONLY the user-import region in both modes —
    // stubs don't map to any source line.
    if let Some(instance) = &doc.instance_script {
        let stub_line_count_ts = count_lines(&s.hoisted[..s.stub_prefix_len.min(s.hoisted.len())]);
        // Overlay cursor starts AFTER the stub block in TS mode (those
        // lines are emitted but not mapped), and at the current line
        // in JS mode (no stubs emitted).
        let mut overlay_cursor =
            current_line(buf.as_str()) + if is_ts { stub_line_count_ts } else { 0 };
        let bytes = s.hoisted.as_bytes();
        let mut byte = s.stub_prefix_len.min(bytes.len());
        for &source_offset in &s.hoisted_byte_offsets {
            // Each hoisted statement runs until either the next `\n`
            // followed by a non-blank line, or the next hoisted
            // offset's projection in the concatenated string. Simplest:
            // count lines until we hit the next statement (we know
            // there's an extra `\n` between statements).
            let stmt_start_byte = byte;
            while byte < bytes.len() && bytes[byte] != b'\n' {
                byte += 1;
            }
            // Multi-line imports: keep walking while indented
            // continuations or close-brace tokens follow.
            while byte < bytes.len() {
                let next = byte + 1;
                if next >= bytes.len() {
                    break;
                }
                let after_nl = bytes[next];
                if after_nl == b'\n' {
                    // Blank line — definitely end of statement.
                    byte += 1;
                    break;
                }
                // Heuristic: if the line starts with an alpha character
                // at column 0, it's a new statement.
                if after_nl.is_ascii_alphabetic() {
                    byte += 1;
                    break;
                }
                byte += 1;
                while byte < bytes.len() && bytes[byte] != b'\n' {
                    byte += 1;
                }
            }
            let stmt_text = &s.hoisted[stmt_start_byte..byte];
            let stmt_line_count = count_lines(stmt_text).max(1);
            let source_line =
                source_line_at(doc.source, instance.content_range.start + source_offset);
            buf.push_line_map(LineMapEntry {
                overlay_start_line: overlay_cursor,
                overlay_end_line: overlay_cursor + stmt_line_count,
                source_start_line: source_line,
            });
            overlay_cursor += stmt_line_count;
            // Skip the trailing newline we wrote.
            if byte < bytes.len() && bytes[byte] == b'\n' {
                byte += 1;
            }
        }
    }
    if is_ts {
        buf.push_str(&s.hoisted);
    } else {
        let user_imports = &s.hoisted[s.stub_prefix_len.min(s.hoisted.len())..];
        buf.push_str(user_imports);
        if !user_imports.ends_with('\n') {
            buf.push_str("\n");
        }
        return;
    }
    if !s.hoisted.ends_with('\n') {
        buf.push_str("\n");
    }
}
