//! Position translation between overlay (line, col) and source
//! (line, col) via line_map / token_map metadata.
//!
//! All public entry points take 1-based (line, col) — matching tsgo's
//! diagnostic output convention. Columns are UTF-16 code units (LSP
//! convention); the helpers convert internally to byte offsets via
//! [`position_to_byte`] and back via [`byte_to_position`].

use svn_emit::{LineMapEntry, TokenMapEntry};

use crate::types::MapData;

/// Translate an overlay `(line, column)` into a byte offset using
/// [`MapData::overlay_line_starts`]. Both line and column are
/// 1-based (matching tsgo's diagnostic output). Returns `None` when
/// the overlay-line-starts table is empty (non-Svelte input) or the
/// requested line is out of range.
pub(crate) fn overlay_byte_offset(data: &MapData, line: u32, column: u32) -> Option<u32> {
    if data.overlay_line_starts.is_empty() || line == 0 {
        return None;
    }
    // tsgo's `column` is 1-based UTF-16 code units; convert via
    // `position_to_byte` so non-ASCII overlay content is handled
    // correctly (the ignore-region filter that consumes this offset
    // would otherwise miss markers when emit-synthesised scaffolding
    // contains multi-byte chars — rare today, but the conversion
    // costs nothing on ASCII-only lines).
    position_to_byte(&data.overlay_line_starts, &data.overlay_text, line, column)
}

/// Translate an overlay line into a source line via the line map.
///
/// The map is sorted by `overlay_start_line`. If `overlay_line` falls
/// inside an entry's range, return the corresponding source line
/// preserving the relative offset. Otherwise return `None` — the
/// diagnostic fired against synthesized scaffolding with no
/// user-source origin and the caller drops it.
pub(crate) fn translate_line(map: &[LineMapEntry], overlay_line: u32) -> Option<u32> {
    if map.is_empty() {
        return None;
    }
    // Find the entry containing overlay_line.
    for entry in map {
        if overlay_line >= entry.overlay_start_line && overlay_line < entry.overlay_end_line {
            let delta = overlay_line - entry.overlay_start_line;
            return Some(entry.source_start_line + delta);
        }
    }
    None
}

/// Translate an overlay `(line, column)` into a source `(line, column)`
/// via [`MapData`]. Both input and output use 1-based line/column.
///
/// Prefers a byte-span [`TokenMapEntry`] when one contains the
/// overlay byte offset corresponding to `(line, column)`. When the
/// overlay position falls inside multiple entries (nested spans), the
/// tightest one wins — that's the one most precisely describing where
/// the user-source content was spliced.
///
/// Falls back to [`translate_line`] on the line number alone when no
/// token-map entry matches; the column is returned unchanged in that
/// case (the line-map covers verbatim script blocks, where overlay
/// column == source column because the script content is emitted
/// verbatim).
///
/// For `identity_map` inputs (Kit overlays) returns `(line, col)`
/// unchanged when neither map covers the position.
pub(crate) fn translate_position(
    data: &MapData,
    overlay_line: u32,
    overlay_col: u32,
) -> Option<(u32, u32)> {
    // Try the token map first — tightest-span wins. Requires a
    // line-starts index to resolve (line, col) → byte offset.
    if !data.token_map.is_empty() && !data.overlay_line_starts.is_empty() {
        if let Some(byte) = position_to_byte(
            &data.overlay_line_starts,
            &data.overlay_text,
            overlay_line,
            overlay_col,
        ) {
            if let Some(entry) = find_tightest_token(&data.token_map, byte) {
                // Preserve the column offset within the span so a
                // diagnostic pointing at the middle of the spliced
                // token still lands at the corresponding position in
                // source. Clamp on overflow — a diagnostic past the
                // source span's end lands at source_byte_end - 1.
                let overlay_offset = byte.saturating_sub(entry.overlay_byte_start);
                let source_byte = entry
                    .source_byte_start
                    .saturating_add(overlay_offset)
                    .min(entry.source_byte_end.saturating_sub(1));
                let (sl, sc) =
                    byte_to_position(&data.source_line_starts, &data.source_text, source_byte);
                return Some((sl, sc));
            }
        }
    }
    // Fall back to the line map. Column is returned unchanged because
    // verbatim script content emits verbatim — overlay column equals
    // source column within a line-map range.
    if let Some(mapped) = translate_line(&data.line_map, overlay_line) {
        return Some((mapped, overlay_col));
    }
    // Identity-map kit files: `kit_inject` splices `: T` annotations on
    // existing lines — overlay never adds lines. Diagnostics against
    // unmodified regions (the common case) line up 1:1 on both axes;
    // on-insertion-line columns may drift but tsgo's diagnostics
    // against kit files are rare and the approximation is better than
    // dropping them entirely.
    if data.identity_map {
        return Some((overlay_line, overlay_col));
    }
    None
}

/// Find the tightest [`TokenMapEntry`] whose overlay byte span
/// contains `byte`. "Tightest" = smallest `overlay_byte_end -
/// overlay_byte_start` span; ties broken by last-wins (later entries
/// reflect deeper nesting when emit pushes parent spans first and
/// child splices second). Returns `None` when no entry covers the
/// byte.
pub(crate) fn find_tightest_token(map: &[TokenMapEntry], byte: u32) -> Option<TokenMapEntry> {
    let mut best: Option<TokenMapEntry> = None;
    for entry in map {
        if byte < entry.overlay_byte_start || byte >= entry.overlay_byte_end {
            continue;
        }
        let width = entry.overlay_byte_end - entry.overlay_byte_start;
        match best {
            None => best = Some(*entry),
            Some(prev) => {
                let prev_width = prev.overlay_byte_end - prev.overlay_byte_start;
                if width <= prev_width {
                    best = Some(*entry);
                }
            }
        }
    }
    best
}

/// Convert a 1-based `(line, UTF-16 col)` into a byte offset.
///
/// tsgo (and upstream svelte-check / TypeScript / LSP) emit
/// **UTF-16 code-unit columns**, NOT byte columns. For pure-ASCII
/// lines the two coincide; for lines containing non-ASCII characters
/// (UTF-8 bytes ≥ 0x80) they diverge — `é` is 1 UTF-16 unit but 2
/// UTF-8 bytes. Walk the line text counting UTF-16 units to land
/// on the correct byte.
///
/// Returns `None` when the line is past EOF. Columns past the end of
/// the line clamp to the line's final byte (matches LSP server
/// behaviour for over-shoots).
pub(crate) fn position_to_byte(
    line_starts: &[u32],
    text: &str,
    line: u32,
    col: u32,
) -> Option<u32> {
    if line == 0 {
        return None;
    }
    let line_idx = (line - 1) as usize;
    if line_idx >= line_starts.len().saturating_sub(1) {
        return None;
    }
    let line_start = line_starts[line_idx];
    let next = line_starts[line_idx + 1];
    if col <= 1 {
        return Some(line_start);
    }
    let target_units = (col - 1) as usize;
    // Walk the line text byte-by-char, counting UTF-16 code units
    // per char (2 for surrogate pairs / supplementary plane, 1
    // otherwise). Stop when we've consumed `target_units` worth.
    let line_bytes_end = next as usize;
    let line_text = match text.get(line_start as usize..line_bytes_end) {
        Some(s) => s,
        // Source bytes don't form a valid UTF-8 slice (shouldn't
        // happen — line_starts is built from str::char_indices via
        // memchr on '\n') — clamp to line end so we still produce a
        // diagnostic at the line, just at column 1.
        None => return Some(line_start),
    };
    let mut units = 0usize;
    for (offset, ch) in line_text.char_indices() {
        if units >= target_units {
            return Some(line_start.saturating_add(offset as u32));
        }
        units = units.saturating_add(ch.len_utf16());
    }
    // Column overshoots the line's end — clamp to the last byte on
    // this line (the newline, if any).
    Some(next.saturating_sub(1).max(line_start))
}

/// Convert a byte offset to a 1-based `(line, UTF-16 col)`.
///
/// Counts UTF-16 code units between the line start and the target
/// byte, mirroring the LSP convention tsgo emits. Pure-ASCII lines
/// pay no extra cost beyond a slice; non-ASCII lines walk char-by-
/// char accumulating `char::len_utf16()`.
///
/// Used to render a matched TokenMapEntry's source byte back into a
/// user-facing position. Clamps to the last line when `byte` is past
/// EOF.
pub(crate) fn byte_to_position(line_starts: &[u32], text: &str, byte: u32) -> (u32, u32) {
    if line_starts.is_empty() {
        return (1, 1);
    }
    // Binary search for the last entry with line_start <= byte.
    let idx = match line_starts.binary_search(&byte) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    // line_starts has a sentinel EOF entry as its last element, so the
    // last real line index is `len - 2`. Clamp here so a byte at EOF
    // doesn't fall on the sentinel and produce a phantom extra line.
    let line_idx = idx.min(line_starts.len().saturating_sub(2));
    let line_start = line_starts[line_idx];
    let line = (line_idx + 1) as u32;
    let line_text = match text.get(line_start as usize..byte as usize) {
        Some(s) => s,
        // Byte didn't land on a UTF-8 boundary, or is past EOF —
        // clamp to column 1.
        None => return (line, 1),
    };
    let mut units = 0u32;
    for ch in line_text.chars() {
        units = units.saturating_add(ch.len_utf16() as u32);
    }
    (line, units + 1)
}
