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
    position_to_byte(
        &data.overlay_line_starts,
        data.overlay_text.get(),
        line,
        column,
    )
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
/// verbatim). A position covered by neither resolves to the user text
/// generated before it on its line — see [`preceding_token_source_byte`].
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
            data.overlay_text.get(),
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
                // Identity-map kinds don't carry a separate source-text
                // copy — the overlay text is the source view, so read the
                // overlay-side line-starts/text for the UTF-16 conversion.
                let (source_line_starts, source_text): (&[u32], &str) = if data.identity_map {
                    (&data.overlay_line_starts, data.overlay_text.get())
                } else {
                    (&data.source_line_starts, &data.source_text)
                };
                let (sl, sc) = byte_to_position(source_line_starts, source_text, source_byte);
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
    if !data.identity_map
        && let Some(byte) = preceding_token_source_byte(data, overlay_line, overlay_col)
    {
        return Some(byte_to_position(
            &data.source_line_starts,
            &data.source_text,
            byte,
        ));
    }
    // Identity-map kit files: `kit_inject` splices `: T` annotations on
    // existing lines and never adds one, so a position moves back by
    // whatever was spliced ahead of it — see `unshift_kit_position`.
    if data.identity_map {
        return Some(unshift_kit_position(data, overlay_line, overlay_col));
    }
    None
}

/// Map a position in generated text to the user text generated just
/// before it on the same overlay line.
///
/// This is how a source-map lookup resolves a position that sits on no
/// mapping of its own: it takes the closest mapping at or before it on
/// that line. Upstream maps tsgo positions exactly this way, so a
/// diagnostic on the generated `)` or wrapper call that follows a
/// spliced expression is reported against that expression rather than
/// dropped. Only a position with no mapped text before it on its line
/// has no source counterpart.
///
/// The answer is the source byte a source map would record there: the
/// last character for text copied byte-for-byte, and the start for
/// text that stands in for its source range.
fn preceding_token_source_byte(data: &MapData, overlay_line: u32, overlay_col: u32) -> Option<u32> {
    if data.token_map.is_empty() || overlay_line == 0 {
        return None;
    }
    let byte = position_to_byte(
        &data.overlay_line_starts,
        data.overlay_text.get(),
        overlay_line,
        overlay_col,
    )?;
    let line_start = *data.overlay_line_starts.get((overlay_line - 1) as usize)?;
    let mut best: Option<TokenMapEntry> = None;
    for entry in &data.token_map {
        if entry.overlay_byte_end > byte
            || entry.overlay_byte_end <= line_start
            || entry.overlay_byte_end == entry.overlay_byte_start
        {
            continue;
        }
        let closer = match best {
            None => true,
            Some(prev) => {
                entry.overlay_byte_end > prev.overlay_byte_end
                    || (entry.overlay_byte_end == prev.overlay_byte_end
                        && entry.overlay_byte_end - entry.overlay_byte_start
                            <= prev.overlay_byte_end - prev.overlay_byte_start)
            }
        };
        if closer {
            best = Some(*entry);
        }
    }
    let entry = best?;
    let copied = entry.overlay_byte_end - entry.overlay_byte_start
        == entry.source_byte_end - entry.source_byte_start;
    Some(if copied {
        entry.source_byte_end - 1
    } else {
        entry.source_byte_start
    })
}

/// Map a kit overlay position back to the user's file the way
/// upstream's `toOriginalPos` does, by subtracting the annotations
/// spliced ahead of it.
///
/// A position strictly *inside* a splice has no source counterpart —
/// the text there is ours — so it collapses to the character the user
/// wrote where the splice went. A position exactly *at* a splice's first
/// character counts that splice as already passed, so it lands the
/// splice's length before that character — possibly on an earlier
/// line, and never before the start of the file.
fn unshift_kit_position(data: &MapData, line: u32, col: u32) -> (u32, u32) {
    let shifts = &data.kit_col_shifts;
    let back = shift_before(shifts, line, col);
    let target = i64::from(col) - i64::from(back);
    if target >= 1 {
        return (line, target as u32);
    }
    // Walk back over the user's earlier lines: one unit reaches the
    // previous line's line break, which sits one past its last column.
    let mut remaining = 1 - target;
    let mut l = line;
    while l > 1 {
        l -= 1;
        let overlay_len = overlay_line_len_utf16(data, l);
        let spliced: u32 = shifts
            .iter()
            .filter(|&&(sl, _, _)| sl == l)
            .map(|&(_, _, len)| len)
            .sum();
        let user_len = i64::from(overlay_len.saturating_sub(spliced));
        if remaining <= user_len + 1 {
            return (l, (user_len + 2 - remaining) as u32);
        }
        remaining -= user_len + 1;
    }
    (1, 1)
}

/// The splice length to subtract for overlay column `col` on `line`.
fn shift_before(shifts: &[(u32, u32, u32)], line: u32, col: u32) -> u32 {
    shifts
        .iter()
        .filter(|&&(shift_line, shift_col, _)| shift_line == line && shift_col <= col)
        .map(|&(_, shift_col, len)| {
            if shift_col < col && shift_col + len > col {
                col - shift_col
            } else {
                len
            }
        })
        .sum()
}

/// UTF-16 length of overlay line `line` (1-based), line break excluded.
fn overlay_line_len_utf16(data: &MapData, line: u32) -> u32 {
    let starts = &data.overlay_line_starts;
    let text = data.overlay_text.get();
    let Some(&start) = starts.get((line - 1) as usize) else {
        return 0;
    };
    let end = starts
        .get(line as usize)
        .map_or(text.len(), |&e| e as usize);
    text.get(start as usize..end)
        .unwrap_or("")
        .trim_end_matches(['\n', '\r'])
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum()
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
/// the line clamp to the start of the next line (the same
/// `nextLineOffset` clamp upstream's `offsetAt` applies to over-shoots).
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
    // Column overshoots the line's end — clamp to the start of the next
    // line (== upstream offsetAt's `nextLineOffset` clamp).
    Some(next.max(line_start))
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

#[cfg(test)]
mod tests {
    use super::{shift_before, translate_position};
    use crate::types::MapData;
    use svn_emit::TokenMapEntry;

    /// Overlay `f(ab);\nxy(ab)` where both `ab`s were copied from source
    /// bytes 10..12, and the second line's `xy(` is generated. Source is
    /// ten filler bytes, `ab`, then more filler, all on one line.
    fn spliced(copied: bool) -> MapData {
        let entry = |start| TokenMapEntry {
            overlay_byte_start: start,
            overlay_byte_end: start + 2,
            source_byte_start: 10,
            source_byte_end: if copied { 12 } else { 11 },
        };
        MapData {
            token_map: vec![entry(2), entry(10)],
            overlay_line_starts: vec![0, 7, 13],
            overlay_text: "f(ab);\nxy(ab)".into(),
            source_line_starts: vec![0, 20],
            source_text: "0123456789ab34567890".into(),
            ..Default::default()
        }
    }

    #[test]
    fn generated_text_after_user_text_resolves_to_its_last_character() {
        // `)` and `;` on line 1 follow the copied `ab`: they report at
        // `b`, source column 12.
        assert_eq!(translate_position(&spliced(true), 1, 5), Some((1, 12)));
        assert_eq!(translate_position(&spliced(true), 1, 6), Some((1, 12)));
        // Inside the copy the column still moves byte for byte.
        assert_eq!(translate_position(&spliced(true), 1, 3), Some((1, 11)));
    }

    #[test]
    fn generated_text_after_a_stand_in_resolves_to_its_start() {
        // A splice that is not a byte-for-byte copy stands in for its
        // whole source range, so what follows reports at its start.
        assert_eq!(translate_position(&spliced(false), 1, 5), Some((1, 11)));
    }

    #[test]
    fn generated_text_with_nothing_before_it_on_the_line_is_dropped() {
        // `xy(` opens line 2; the copy on line 1 does not reach it.
        assert_eq!(translate_position(&spliced(true), 2, 1), None);
        assert_eq!(translate_position(&spliced(true), 2, 3), None);
        // The `)` after line 2's own copy resolves again.
        assert_eq!(translate_position(&spliced(true), 2, 6), Some((1, 12)));
    }

    /// The two splices `kit_inject` makes in
    ///
    ///     export const handle = async ({ event, resolve }) => {
    ///
    /// against SvelteKit 3: a 53-unit parameter annotation at column 48
    /// (just past the destructure's `}`), then a 51-unit return
    /// annotation at column 103 (where `=>` sat, pushed right by the
    /// first splice). Measured from a real overlay, not invented.
    const HOOK_SPLICES: [(u32, u32, u32); 2] = [(1, 48, 53), (1, 103, 51)];

    /// The column a same-line position maps to (the result stays on the
    /// line for every case below).
    fn unshift_column_for_test(line: u32, col: u32) -> u32 {
        col - shift_before(&HOOK_SPLICES, line, col)
    }

    #[test]
    fn columns_before_every_splice_are_untouched() {
        assert_eq!(unshift_column_for_test(1, 1), 1);
        assert_eq!(unshift_column_for_test(1, 32), 32);
    }

    #[test]
    fn a_column_past_both_splices_loses_both_lengths() {
        // Overlay column 105 is the `ReturnType` the async-return
        // diagnostic fires on; the user wrote `=>` at column 50 there,
        // which is where upstream reports it.
        assert_eq!(unshift_column_for_test(1, 105), 50);
    }

    #[test]
    fn a_column_between_the_splices_loses_only_the_first() {
        // Overlay column 101 is the `)` closing the parameter list.
        assert_eq!(unshift_column_for_test(1, 101), 48);
    }

    #[test]
    fn columns_inside_a_splice_collapse_to_where_it_starts() {
        // There is no user text under a splice, so the honest answer is
        // the character the user wrote at that point.
        assert_eq!(unshift_column_for_test(1, 60), 48);
        assert_eq!(unshift_column_for_test(1, 100), 48);
    }

    #[test]
    fn a_column_at_a_splice_start_counts_the_splice_as_passed() {
        // Upstream's `toOriginalPos` subtracts a splice whose first
        // character the position sits on, so the column lands the
        // splice's length before the user's character.
        assert_eq!(shift_before(&HOOK_SPLICES, 1, 103), 53 + 51);
    }

    #[test]
    fn a_splice_start_position_can_move_to_an_earlier_line() {
        // `export function load({ url } = {}) {` on line 2, with a
        // 38-unit annotation at overlay column 34. A diagnostic at the
        // annotation's first character lands 38 units before column 34:
        // 33 units back reaches line 2's start, 1 more is line 1's
        // break (column 11 after `// comment`), and 4 more is column 7.
        let source = "// comment\nexport function load({ url } = {}: import('./$types.js').PageLoadEvent) {}\n";
        let mut data = MapData {
            overlay_line_starts: svn_emit::compute_line_starts(source),
            overlay_text: source.into(),
            identity_map: true,
            kit_col_shifts: vec![(2, 34, 38)],
            ..Default::default()
        };
        assert_eq!(super::unshift_kit_position(&data, 2, 34), (1, 7));
        // Past the start of the file it clamps.
        data.kit_col_shifts = vec![(2, 34, 80)];
        assert_eq!(super::unshift_kit_position(&data, 2, 34), (1, 1));
        // Inside the splice: where it went.
        assert_eq!(super::unshift_kit_position(&data, 2, 40), (2, 34));
    }

    #[test]
    fn only_splices_on_the_same_line_apply() {
        assert_eq!(unshift_column_for_test(2, 105), 105);
    }
}
