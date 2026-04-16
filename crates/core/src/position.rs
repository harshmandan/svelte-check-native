//! Line/column positions derived from byte offsets.
//!
//! Construct one `PositionMap` per source file; all offset→position queries
//! run against it via binary search on a precomputed line-starts table.
//!
//! ### Convention
//!
//! `Position` is **0-based** in both line and column, matching LSP
//! (`vscode-languageserver-types`). The `machine-verbose` output format uses
//! this form directly. The `machine`, `human`, and `human-verbose` formats
//! convert to 1-based at the formatter boundary.
//!
//! Column is counted in **UTF-16 code units** — also LSP convention. For
//! ASCII-only source this equals the byte column; for non-ASCII content it
//! diverges. Byte columns are available separately via [`PositionMap::line_col_utf8`].

use crate::range::Range;

/// A 0-based line + UTF-16 column position in a source file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Position {
    pub line: u32,
    /// UTF-16 code-unit column (LSP convention).
    pub character: u32,
}

/// Byte-offset → line/column resolver, built once per source file.
///
/// Construction is linear in source length (fast — uses `memchr` to find
/// newlines). Queries are `O(log lines)`.
#[derive(Debug)]
pub struct PositionMap<'src> {
    source: &'src str,
    /// Byte offset of the first character on each line.
    /// `line_starts[0] == 0`; `line_starts.len() == line_count`.
    line_starts: Vec<u32>,
}

impl<'src> PositionMap<'src> {
    /// Build a position map for the given source.
    pub fn new(source: &'src str) -> Self {
        let bytes = source.as_bytes();
        // Heuristic capacity: assume ~40-char lines on average.
        let mut line_starts = Vec::with_capacity(bytes.len() / 40 + 1);
        line_starts.push(0);
        for pos in memchr::memchr_iter(b'\n', bytes) {
            // Next line starts *after* the newline.
            line_starts.push((pos + 1) as u32);
        }
        Self {
            source,
            line_starts,
        }
    }

    /// Number of lines, counting a trailing newline as starting a new (empty)
    /// line. An empty source has line count 1.
    #[inline]
    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// Byte length of the underlying source.
    #[inline]
    pub fn source_len(&self) -> u32 {
        self.source.len() as u32
    }

    /// Convert a byte offset to LSP-style (0-based line, 0-based UTF-16 char).
    ///
    /// Offsets beyond the source length are clamped to the end.
    pub fn position_of(&self, offset: u32) -> Position {
        let clamped = offset.min(self.source_len());
        let line_idx = self.line_index_of_offset(clamped);
        let line_start = self.line_starts[line_idx as usize];
        // Byte slice from line start to the queried offset. These indices are
        // guaranteed to land on UTF-8 char boundaries because line_start is
        // either 0 or points just past a `\n` (always a boundary), and offsets
        // come from well-formed parses.
        let slice = &self.source[line_start as usize..clamped as usize];
        let character = count_utf16_units(slice);
        Position {
            line: line_idx,
            character,
        }
    }

    /// Convert a byte offset to (0-based line, 0-based UTF-8 byte column).
    ///
    /// Useful for internal diagnostics that don't need LSP UTF-16 accounting.
    pub fn line_col_utf8(&self, offset: u32) -> (u32, u32) {
        let clamped = offset.min(self.source_len());
        let line_idx = self.line_index_of_offset(clamped);
        let line_start = self.line_starts[line_idx as usize];
        (line_idx, clamped - line_start)
    }

    /// Convert a `Range` to a pair of `Position`s. Convenience wrapper.
    #[inline]
    pub fn range_positions(&self, range: Range) -> (Position, Position) {
        (self.position_of(range.start), self.position_of(range.end))
    }

    /// Index into `line_starts` for the line containing `offset` (clamped).
    fn line_index_of_offset(&self, offset: u32) -> u32 {
        // `line_starts` is strictly increasing; binary search gives us the
        // line whose start <= offset. On Ok match the offset IS a line start.
        match self.line_starts.binary_search(&offset) {
            Ok(idx) => idx as u32,
            // `idx` here is the insertion point; the offset falls *inside*
            // the line that starts at line_starts[idx-1]. saturating_sub(1)
            // guards against the theoretically impossible idx==0 case
            // (line_starts[0] == 0 and offset >= 0, so binary_search always
            // returns Ok(0) for offset==0).
            Err(idx) => idx.saturating_sub(1) as u32,
        }
    }
}

/// Count UTF-16 code units in a UTF-8 slice.
///
/// Any code point in the BMP (U+0000..=U+FFFF) is 1 UTF-16 unit; anything in
/// supplementary planes (U+10000+) is 2. Classified by UTF-8 byte width:
/// - 1 byte (ASCII) → 1 unit
/// - 2 bytes (U+0080..=U+07FF) → 1 unit
/// - 3 bytes (U+0800..=U+FFFF) → 1 unit
/// - 4 bytes (U+10000..=U+10FFFF) → 2 units
///
/// We branch only on the leading byte of each UTF-8 sequence.
#[inline]
fn count_utf16_units(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut units: u32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x80 {
            // ASCII fast path.
            units += 1;
            i += 1;
        } else if b < 0xC0 {
            // Continuation byte — shouldn't be leading but be safe.
            i += 1;
        } else if b < 0xE0 {
            // 2-byte sequence.
            units += 1;
            i += 2;
        } else if b < 0xF0 {
            // 3-byte sequence.
            units += 1;
            i += 3;
        } else {
            // 4-byte sequence — surrogate pair in UTF-16.
            units += 2;
            i += 4;
        }
    }
    units
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source_has_one_line() {
        let pm = PositionMap::new("");
        assert_eq!(pm.line_count(), 1);
        assert_eq!(
            pm.position_of(0),
            Position {
                line: 0,
                character: 0
            }
        );
    }

    #[test]
    fn single_line_no_newline() {
        let pm = PositionMap::new("hello");
        assert_eq!(pm.line_count(), 1);
        assert_eq!(
            pm.position_of(0),
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(
            pm.position_of(5),
            Position {
                line: 0,
                character: 5
            }
        );
    }

    #[test]
    fn two_lines() {
        let src = "ab\ncd";
        let pm = PositionMap::new(src);
        assert_eq!(pm.line_count(), 2);
        assert_eq!(
            pm.position_of(0),
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(
            pm.position_of(1),
            Position {
                line: 0,
                character: 1
            }
        );
        assert_eq!(
            pm.position_of(2),
            Position {
                line: 0,
                character: 2
            }
        ); // the '\n'
        assert_eq!(
            pm.position_of(3),
            Position {
                line: 1,
                character: 0
            }
        );
        assert_eq!(
            pm.position_of(4),
            Position {
                line: 1,
                character: 1
            }
        );
        assert_eq!(
            pm.position_of(5),
            Position {
                line: 1,
                character: 2
            }
        );
    }

    #[test]
    fn trailing_newline_starts_new_empty_line() {
        let pm = PositionMap::new("abc\n");
        assert_eq!(pm.line_count(), 2);
        // Offset 4 is beyond the newline (start of the trailing empty line).
        assert_eq!(
            pm.position_of(4),
            Position {
                line: 1,
                character: 0
            }
        );
    }

    #[test]
    fn clamps_offset_beyond_source() {
        let pm = PositionMap::new("abc");
        assert_eq!(
            pm.position_of(1_000),
            Position {
                line: 0,
                character: 3
            }
        );
    }

    #[test]
    fn line_col_utf8_works() {
        let pm = PositionMap::new("ab\ncdef");
        assert_eq!(pm.line_col_utf8(0), (0, 0));
        assert_eq!(pm.line_col_utf8(2), (0, 2));
        assert_eq!(pm.line_col_utf8(3), (1, 0));
        assert_eq!(pm.line_col_utf8(5), (1, 2));
    }

    #[test]
    fn range_positions_returns_pair() {
        let pm = PositionMap::new("hello\nworld");
        let r = Range::new(2, 8);
        let (a, b) = pm.range_positions(r);
        assert_eq!(
            a,
            Position {
                line: 0,
                character: 2
            }
        );
        assert_eq!(
            b,
            Position {
                line: 1,
                character: 2
            }
        );
    }

    #[test]
    fn utf16_ascii_equals_bytes() {
        assert_eq!(count_utf16_units(""), 0);
        assert_eq!(count_utf16_units("abc"), 3);
    }

    #[test]
    fn utf16_two_byte_sequence() {
        // é is U+00E9 — 2 UTF-8 bytes, 1 UTF-16 unit.
        assert_eq!(count_utf16_units("café"), 4);
    }

    #[test]
    fn utf16_three_byte_sequence() {
        // あ is U+3042 — 3 UTF-8 bytes, 1 UTF-16 unit.
        assert_eq!(count_utf16_units("あ"), 1);
        assert_eq!(count_utf16_units("xあy"), 3);
    }

    #[test]
    fn utf16_four_byte_sequence_is_surrogate_pair() {
        // 🎉 is U+1F389 — 4 UTF-8 bytes, 2 UTF-16 units (surrogate pair).
        assert_eq!(count_utf16_units("🎉"), 2);
        assert_eq!(count_utf16_units("a🎉b"), 4);
    }

    #[test]
    fn position_in_unicode_line() {
        // "café\nfoo" — café takes bytes 0..5 (c=1, a=1, f=1, é=2), newline at 5.
        let src = "café\nfoo";
        let pm = PositionMap::new(src);
        // Offset at 'é' start (byte 3) → 0, character 3 (utf-16 units c,a,f).
        assert_eq!(
            pm.position_of(3),
            Position {
                line: 0,
                character: 3
            }
        );
        // Offset just after 'é' (byte 5) → 0, character 4.
        assert_eq!(
            pm.position_of(5),
            Position {
                line: 0,
                character: 4
            }
        );
        // Start of line 2.
        assert_eq!(
            pm.position_of(6),
            Position {
                line: 1,
                character: 0
            }
        );
    }

    #[test]
    fn position_with_emoji_surrogate() {
        // "a🎉b" — a=1 byte, 🎉=4 bytes, b=1 byte.
        let src = "a🎉b";
        let pm = PositionMap::new(src);
        assert_eq!(
            pm.position_of(0),
            Position {
                line: 0,
                character: 0
            }
        ); // before a
        assert_eq!(
            pm.position_of(1),
            Position {
                line: 0,
                character: 1
            }
        ); // after a, before 🎉
        assert_eq!(
            pm.position_of(5),
            Position {
                line: 0,
                character: 3
            }
        ); // after 🎉 (surrogate pair = 2 units)
        assert_eq!(
            pm.position_of(6),
            Position {
                line: 0,
                character: 4
            }
        ); // after b
    }
}
