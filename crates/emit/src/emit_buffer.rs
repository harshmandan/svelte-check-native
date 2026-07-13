//! Position-tracking emit buffer.
//!
//! Phase 4 / R2 of `notes/PLAN.md`: a small, minimal abstraction over
//! the `String` buffer + `Vec<LineMapEntry>` + `Vec<TokenMapEntry>`
//! pattern emit has threaded through every entry point. The buffer
//! knows how to append verbatim regions from the user's source (and
//! synthesises the LineMapEntry automatically) and how to splice
//! synthesized content with an optional source anchor (pushing a
//! TokenMapEntry).
//!
//! Replaces manual bookkeeping of the form:
//!
//! ```rust,ignore
//! let overlay_line = current_line(&out);
//! let source_line = source_line_at(doc.source, content_range.start);
//! let line_count = count_lines(content);
//! out.push_str(content);
//! if line_count > 0 {
//!     line_map.push(LineMapEntry {
//!         overlay_start_line: overlay_line,
//!         overlay_end_line: overlay_line + line_count,
//!         source_start_line: source_line,
//!     });
//! }
//! ```
//!
//! …with one call:
//!
//! ```rust,ignore
//! buf.append_verbatim(content, source, content_range);
//! ```
//!
//! The buffer isn't a full MagicString clone — we don't need
//! `overwrite` / `move` since our emit doesn't MUTATE user source, it
//! generates new TS that incorporates verbatim regions. This is the
//! smaller shape that actually fits our use case.
//!
use std::fmt;

use svn_core::Range;

use crate::{LineMapEntry, TokenMapEntry};

/// Position-tracking emit buffer. See module docs.
pub(crate) struct EmitBuffer {
    out: String,
    /// Current overlay line (1-based). Incremented by
    /// `append_*` calls based on newline count in the appended text.
    /// Reads return the line the NEXT append would start on.
    overlay_line: u32,
    line_map: Vec<LineMapEntry>,
    token_map: Vec<TokenMapEntry>,
}

impl EmitBuffer {
    /// Create a buffer with the given initial capacity. `capacity`
    /// should approximate the final overlay size — oversizing wastes
    /// memory, undersizing causes realloc churn on every push.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            out: String::with_capacity(capacity),
            overlay_line: 1,
            line_map: Vec::new(),
            token_map: Vec::new(),
        }
    }

    /// Current length of the underlying buffer (bytes).
    pub fn len(&self) -> usize {
        self.out.len()
    }

    /// Current overlay line (1-based). Equal to the line the next
    /// `append_*` call would start on. Only used by the buffer's own
    /// tests today — production callsites read via `as_str()` +
    /// `current_line(s)` (the free function in `lib.rs`) when they
    /// want an authoritative scan, or skip the counter entirely.
    #[cfg(test)]
    pub fn current_line(&self) -> u32 {
        self.overlay_line
    }

    /// Peek at the accumulated buffer. Useful during tests and for
    /// sites that still need to scan their own output.
    pub fn as_str(&self) -> &str {
        &self.out
    }

    /// Append synthesized text with no source anchor.
    ///
    /// Use for scaffolding lines emit generates from whole cloth —
    /// `async function $$render_<hash>() {`, `void (...);` blocks,
    /// the generated `declare const __svn_component_default`, etc.
    /// Diagnostics falling inside these lines have no source
    /// position; the mapper clamps them to the nearest preceding
    /// LineMapEntry.
    pub fn append_synthetic(&mut self, text: &str) {
        self.out.push_str(text);
        self.overlay_line += count_newlines(text);
    }

    /// Alias for [`append_synthetic`] matching the `String::push_str`
    /// signature. Offered so call sites doing bulk `out.push_str(...)`
    /// can migrate to `buf.push_str(...)` with zero shape change.
    pub fn push_str(&mut self, text: &str) {
        self.append_synthetic(text);
    }

    /// Append a single character. Matches `String::push`'s signature so
    /// char-at-a-time call sites (e.g. escape-encoding a JS string
    /// literal) migrate without structural churn. Updates the overlay
    /// line counter if the char is `\n`.
    pub fn push(&mut self, ch: char) {
        self.out.push(ch);
        if ch == '\n' {
            self.overlay_line += 1;
        }
    }

    /// Append text verbatim from `source[source_range]`, recording
    /// a [`LineMapEntry`] that maps the overlay lines back to the
    /// corresponding source lines.
    ///
    /// `text` SHOULD equal `source.get(source_range.start..source_range.end)`.
    /// The helper accepts `text` separately so callers that have
    /// already pre-processed the slice (e.g. normalising trailing
    /// newlines) don't need to re-slice.
    ///
    /// A LineMapEntry is pushed only when the region spans one or
    /// more lines; zero-line appends (in-line fragments) get no
    /// entry — they don't have a distinct overlay-line range to
    /// map.
    pub fn append_verbatim(&mut self, text: &str, source: &str, source_range: Range) {
        let overlay_start = self.overlay_line;
        let line_count = count_newlines(text);
        self.out.push_str(text);
        self.overlay_line += line_count;
        if line_count > 0 {
            let source_start_line = line_number_at(source, source_range.start);
            self.line_map.push(LineMapEntry {
                overlay_start_line: overlay_start,
                overlay_end_line: overlay_start + line_count,
                source_start_line,
            });
        }
    }

    /// Append synthesized text and record a [`TokenMapEntry`] that
    /// maps its overlay byte span back to `source_range`.
    ///
    /// Use at splice sites where a synthesized fragment exists to
    /// diagnose a specific user-source position — template `{expr}`
    /// interpolations, `bind:this={x}` assignments, etc. Without a
    /// TokenMapEntry the diagnostic mapper falls back to the
    /// coarser line-map range or drops the diagnostic entirely.
    pub fn append_with_source(&mut self, text: &str, source_range: Range) {
        let overlay_byte_start = self.out.len() as u32;
        self.out.push_str(text);
        let overlay_byte_end = self.out.len() as u32;
        self.overlay_line += count_newlines(text);
        self.token_map.push(TokenMapEntry {
            overlay_byte_start,
            overlay_byte_end,
            source_byte_start: source_range.start,
            source_byte_end: source_range.end,
        });
    }

    /// Push a pre-computed [`LineMapEntry`]. Escape hatch for
    /// migration sites that build the entry differently than
    /// `append_verbatim`'s contract.
    pub fn push_line_map(&mut self, entry: LineMapEntry) {
        self.line_map.push(entry);
    }

    /// Push a pre-computed [`TokenMapEntry`]. Escape hatch, same
    /// rationale as [`push_line_map`].
    pub fn push_token_map(&mut self, entry: TokenMapEntry) {
        self.token_map.push(entry);
    }

    /// Direct access to the underlying `String` for sites not yet
    /// ported to the new API. Intentionally offered so migration
    /// can happen incrementally — appends here don't go through
    /// the line-map-tracking path.
    pub fn raw_string_mut(&mut self) -> &mut String {
        &mut self.out
    }

    /// Refresh `current_line` by scanning the whole buffer. Used by
    /// migration sites that bypass `append_*` and push directly into
    /// the raw string. Callers should migrate to `append_*` when
    /// possible.
    pub fn resync_current_line(&mut self) {
        self.overlay_line = 1 + count_newlines(&self.out);
    }

    /// Re-anchor token-map entries after an in-place rewrite spliced
    /// bytes into the buffer via `raw_string_mut()`.
    ///
    /// `insertions` are ascending `(position, length)` pairs in the
    /// PRE-insertion buffer coordinates — the coordinate space every
    /// existing entry was recorded in. Without this fix-up, an
    /// insertion inside or before an already-mapped span makes every
    /// later diagnostic reverse-map to the wrong column (or line),
    /// because position translation interpolates byte offsets inside
    /// the span.
    ///
    /// Per entry:
    /// - entirely before the first insertion → unchanged;
    /// - at/after an insertion point → shifted right by the inserted
    ///   length (an insertion AT the start lands before the entry);
    /// - spanning an insertion point:
    ///   - a 1:1 verbatim entry (overlay span length == source span
    ///     length, e.g. the script-body entry) is SPLIT at the point
    ///     so both halves keep their exact byte-for-byte overlay →
    ///     source correspondence;
    ///   - a non-1:1 entry (synthesized text anchored to a source
    ///     range) just grows by the inserted length — it's a single
    ///     anchor, not a positional map, so splitting has nothing to
    ///     preserve.
    ///
    /// The line map needs no adjustment: it's line-based and the
    /// in-place rewrites never insert newlines (their insertion-length
    /// accounting would drift the buffer's line counter otherwise —
    /// callers own that invariant).
    pub fn adjust_token_map_for_insertions(&mut self, insertions: &[(u32, u32)]) {
        if insertions.is_empty() || self.token_map.is_empty() {
            return;
        }
        let mut adjusted: Vec<TokenMapEntry> =
            Vec::with_capacity(self.token_map.len() + insertions.len());
        for entry in &self.token_map {
            let one_to_one = entry.overlay_byte_end - entry.overlay_byte_start
                == entry.source_byte_end - entry.source_byte_start;
            // Segment the entry at each insertion strictly inside it
            // (1:1 entries only). Each segment is (overlay_start,
            // overlay_end, source_start) in pre-insertion coordinates.
            let mut segments: Vec<(u32, u32, u32)> = Vec::new();
            let mut seg_start = entry.overlay_byte_start;
            let mut seg_src = entry.source_byte_start;
            if one_to_one {
                for &(pos, _) in insertions {
                    if pos > seg_start && pos < entry.overlay_byte_end {
                        segments.push((seg_start, pos, seg_src));
                        seg_src += pos - seg_start;
                        seg_start = pos;
                    }
                }
            }
            segments.push((seg_start, entry.overlay_byte_end, seg_src));
            for (start, end, src_start) in segments {
                // Shift by everything inserted at or before the
                // segment's start; the segment interior holds no
                // insertion point (guaranteed by the split above for
                // 1:1 entries), so start and end shift together. A
                // non-1:1 entry keeps its start shift but its end
                // additionally absorbs interior insertions (the
                // entry grows over them).
                let shift: u32 = insertions
                    .iter()
                    .take_while(|&&(pos, _)| pos <= start)
                    .map(|&(_, len)| len)
                    .sum();
                let end_shift: u32 = insertions
                    .iter()
                    .take_while(|&&(pos, _)| pos < end || pos <= start)
                    .map(|&(_, len)| len)
                    .sum();
                adjusted.push(TokenMapEntry {
                    overlay_byte_start: start + shift,
                    overlay_byte_end: end + end_shift,
                    source_byte_start: src_start,
                    source_byte_end: if one_to_one {
                        src_start + (end - start)
                    } else {
                        entry.source_byte_end
                    },
                });
            }
        }
        self.token_map = adjusted;
    }

    /// Consume the buffer and return its parts.
    pub fn finish(self) -> (String, Vec<LineMapEntry>, Vec<TokenMapEntry>) {
        (self.out, self.line_map, self.token_map)
    }
}

/// `fmt::Write` routes through `append_synthetic` so the overlay-line
/// counter stays in sync when call sites use `write!(buf, ...)` /
/// `writeln!(buf, ...)` macros. Formatted fragments are treated as
/// synthesized content with no source anchor — use
/// [`EmitBuffer::append_with_source`] when a fragment needs a
/// TokenMapEntry.
impl fmt::Write for EmitBuffer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.append_synthetic(s);
        Ok(())
    }
}

fn count_newlines(s: &str) -> u32 {
    s.bytes().filter(|&b| b == b'\n').count() as u32
}

/// Return the 1-based line number of `byte_offset` in `source`.
/// Mirrors `source_line_at` in `util.rs` but is kept here so this
/// module is self-contained.
fn line_number_at(source: &str, byte_offset: u32) -> u32 {
    let cap = byte_offset as usize;
    1 + source
        .as_bytes()
        .iter()
        .take(cap.min(source.len()))
        .filter(|&&b| b == b'\n')
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_starts_at_line_one() {
        let buf = EmitBuffer::with_capacity(0);
        assert_eq!(buf.current_line(), 1);
        assert_eq!(buf.as_str(), "");
    }

    #[test]
    fn append_synthetic_advances_line_on_newline() {
        let mut buf = EmitBuffer::with_capacity(16);
        buf.append_synthetic("hello\nworld\n");
        assert_eq!(buf.current_line(), 3);
        let (out, lm, tm) = buf.finish();
        assert_eq!(out, "hello\nworld\n");
        assert!(lm.is_empty());
        assert!(tm.is_empty());
    }

    #[test]
    fn append_verbatim_records_line_map_for_multi_line_region() {
        // source layout (line-numbered):
        //   1: const a = 1;
        //   2: const b = 2;
        //   3: const c = 3;
        // overlay grows:
        //   1: // header\n
        //   2: const a = 1;\n
        //   3: const b = 2;\n
        //   4: const c = 3;\n
        let source = "const a = 1;\nconst b = 2;\nconst c = 3;\n";
        let mut buf = EmitBuffer::with_capacity(64);
        buf.append_synthetic("// header\n");
        buf.append_verbatim(source, source, Range::new(0, source.len() as u32));
        let (out, lm, tm) = buf.finish();
        assert_eq!(out, "// header\nconst a = 1;\nconst b = 2;\nconst c = 3;\n");
        assert_eq!(lm.len(), 1);
        assert_eq!(lm[0].overlay_start_line, 2);
        assert_eq!(lm[0].overlay_end_line, 5);
        assert_eq!(lm[0].source_start_line, 1);
        assert!(tm.is_empty());
    }

    #[test]
    fn append_verbatim_no_newline_no_line_map_entry() {
        // Inline fragment (no \n) — no distinct overlay-line range.
        let source = "foo";
        let mut buf = EmitBuffer::with_capacity(8);
        buf.append_verbatim(source, source, Range::new(0, 3));
        let (_, lm, _) = buf.finish();
        assert!(lm.is_empty());
    }

    #[test]
    fn append_verbatim_source_start_line_respects_offset() {
        // The region begins at source line 3 (two \n before offset).
        let source = "line1\nline2\nline3content\n";
        let line3_start = 12u32; // after the two \n
        let line3_content = "line3content\n";
        let mut buf = EmitBuffer::with_capacity(32);
        buf.append_verbatim(
            line3_content,
            source,
            Range::new(line3_start, source.len() as u32),
        );
        let (_, lm, _) = buf.finish();
        assert_eq!(lm.len(), 1);
        assert_eq!(lm[0].source_start_line, 3);
    }

    #[test]
    fn append_with_source_records_token_map() {
        let mut buf = EmitBuffer::with_capacity(32);
        buf.append_synthetic("/*svn_I*/");
        buf.append_with_source("(foo)", Range::new(10, 13));
        let (out, _lm, tm) = buf.finish();
        assert_eq!(out, "/*svn_I*/(foo)");
        assert_eq!(tm.len(), 1);
        assert_eq!(tm[0].overlay_byte_start, 9); // after "/*svn_I*/"
        assert_eq!(tm[0].overlay_byte_end, 14);
        assert_eq!(tm[0].source_byte_start, 10);
        assert_eq!(tm[0].source_byte_end, 13);
    }

    #[test]
    fn append_with_source_advances_line_counter() {
        let mut buf = EmitBuffer::with_capacity(32);
        buf.append_with_source("a\nb", Range::new(0, 3));
        assert_eq!(buf.current_line(), 2);
    }

    #[test]
    fn resync_from_raw_string_mut_matches_reality() {
        // Escape hatch: caller pushes via raw_string_mut, then
        // resync to recover the correct line count.
        let mut buf = EmitBuffer::with_capacity(16);
        buf.append_synthetic("one\n");
        buf.raw_string_mut().push_str("two\nthree\n");
        buf.resync_current_line();
        assert_eq!(buf.current_line(), 4);
    }

    #[test]
    fn push_char_advances_line_on_newline() {
        let mut buf = EmitBuffer::with_capacity(8);
        buf.push('a');
        buf.push('\n');
        buf.push('b');
        assert_eq!(buf.as_str(), "a\nb");
        assert_eq!(buf.current_line(), 2);
    }

    #[test]
    fn push_str_alias_routes_through_append_synthetic() {
        let mut buf = EmitBuffer::with_capacity(16);
        buf.push_str("alpha\nbeta\n");
        assert_eq!(buf.current_line(), 3);
        assert_eq!(buf.as_str(), "alpha\nbeta\n");
    }

    #[test]
    fn fmt_write_advances_line_counter() {
        use std::fmt::Write as _;
        let mut buf = EmitBuffer::with_capacity(32);
        write!(buf, "const x = {};", 42).unwrap();
        writeln!(buf).unwrap();
        writeln!(buf, "const y = {};", 7).unwrap();
        assert_eq!(buf.as_str(), "const x = 42;\nconst y = 7;\n");
        assert_eq!(buf.current_line(), 3);
        let (_, lm, tm) = buf.finish();
        assert!(lm.is_empty());
        assert!(tm.is_empty());
    }

    #[test]
    fn adjust_insertions_splits_one_to_one_entry() {
        // Verbatim script-body entry: overlay [10, 30) ↔ source
        // [100, 120). An 8-byte insertion at overlay byte 20 must
        // split it so bytes before 20 keep their mapping and bytes
        // after map from source 110 onward (shifted right by 8).
        let mut buf = EmitBuffer::with_capacity(0);
        buf.push_token_map(TokenMapEntry {
            overlay_byte_start: 10,
            overlay_byte_end: 30,
            source_byte_start: 100,
            source_byte_end: 120,
        });
        buf.adjust_token_map_for_insertions(&[(20, 8)]);
        let (_, _, tm) = buf.finish();
        assert_eq!(tm.len(), 2);
        assert_eq!(
            tm[0],
            TokenMapEntry {
                overlay_byte_start: 10,
                overlay_byte_end: 20,
                source_byte_start: 100,
                source_byte_end: 110,
            }
        );
        assert_eq!(
            tm[1],
            TokenMapEntry {
                overlay_byte_start: 28,
                overlay_byte_end: 38,
                source_byte_start: 110,
                source_byte_end: 120,
            }
        );
    }

    #[test]
    fn adjust_insertions_shifts_entry_after_point() {
        // Entry fully after the insertion shifts; entry fully before
        // is untouched; insertion exactly at an entry's end doesn't
        // move that end.
        let mut buf = EmitBuffer::with_capacity(0);
        buf.push_token_map(TokenMapEntry {
            overlay_byte_start: 0,
            overlay_byte_end: 5,
            source_byte_start: 40,
            source_byte_end: 45,
        });
        buf.push_token_map(TokenMapEntry {
            overlay_byte_start: 5,
            overlay_byte_end: 9,
            source_byte_start: 50,
            source_byte_end: 54,
        });
        buf.adjust_token_map_for_insertions(&[(5, 3)]);
        let (_, _, tm) = buf.finish();
        assert_eq!(tm[0].overlay_byte_start, 0);
        assert_eq!(tm[0].overlay_byte_end, 5);
        assert_eq!(tm[1].overlay_byte_start, 8);
        assert_eq!(tm[1].overlay_byte_end, 12);
        assert_eq!(tm[1].source_byte_start, 50);
    }

    #[test]
    fn adjust_insertions_grows_anchor_entry() {
        // Non-1:1 entry (synthesized text anchored to a source range):
        // an interior insertion grows the overlay span, source range
        // unchanged — it's an anchor, not a positional map.
        let mut buf = EmitBuffer::with_capacity(0);
        buf.push_token_map(TokenMapEntry {
            overlay_byte_start: 10,
            overlay_byte_end: 20,
            source_byte_start: 100,
            source_byte_end: 103,
        });
        buf.adjust_token_map_for_insertions(&[(15, 4)]);
        let (_, _, tm) = buf.finish();
        assert_eq!(tm.len(), 1);
        assert_eq!(
            tm[0],
            TokenMapEntry {
                overlay_byte_start: 10,
                overlay_byte_end: 24,
                source_byte_start: 100,
                source_byte_end: 103,
            }
        );
    }

    #[test]
    fn adjust_insertions_multiple_points_accumulate() {
        // Two insertions inside one 1:1 entry: three segments, each
        // shifted by the total length inserted before it.
        let mut buf = EmitBuffer::with_capacity(0);
        buf.push_token_map(TokenMapEntry {
            overlay_byte_start: 0,
            overlay_byte_end: 30,
            source_byte_start: 0,
            source_byte_end: 30,
        });
        buf.adjust_token_map_for_insertions(&[(10, 2), (20, 5)]);
        let (_, _, tm) = buf.finish();
        assert_eq!(tm.len(), 3);
        assert_eq!((tm[0].overlay_byte_start, tm[0].overlay_byte_end), (0, 10));
        assert_eq!((tm[1].overlay_byte_start, tm[1].overlay_byte_end), (12, 22));
        assert_eq!((tm[2].overlay_byte_start, tm[2].overlay_byte_end), (27, 37));
        assert_eq!(tm[1].source_byte_start, 10);
        assert_eq!(tm[2].source_byte_start, 20);
        assert_eq!(tm[2].source_byte_end, 30);
    }

    #[test]
    fn push_line_map_preserves_prior_overlay_position() {
        let mut buf = EmitBuffer::with_capacity(16);
        buf.append_synthetic("x\n");
        buf.push_line_map(LineMapEntry {
            overlay_start_line: 42,
            overlay_end_line: 43,
            source_start_line: 7,
        });
        let (_, lm, _) = buf.finish();
        assert_eq!(lm.len(), 1);
        assert_eq!(lm[0].overlay_start_line, 42);
    }
}
