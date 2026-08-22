//! Translating a cursor between a `.svelte` file and its overlay.
//!
//! Diagnostics only ever travel overlay → source, and `svn_typecheck` already
//! does that. A hover needs the other direction: the editor gives a position in
//! the `.svelte` file and tsgo has to be asked about the corresponding place in
//! the generated TypeScript. Both emit maps invert cleanly, so this is a
//! lookup, not a heuristic.
//!
//! Columns are treated as bytes rather than UTF-16 code units. That is exact
//! for ASCII source and wrong past the first non-ASCII character on a line —
//! a spike-level shortcut, and the first thing to fix if this grows up.

use svn_emit::{LineMapEntry, TokenMapEntry};

/// Everything needed to move a position between one source file and its
/// overlay, captured when the file was last emitted.
#[derive(Debug, Clone, Default)]
pub struct FileMaps {
    pub line_map: Vec<LineMapEntry>,
    pub token_map: Vec<TokenMapEntry>,
    pub overlay_line_starts: Vec<u32>,
    pub source_line_starts: Vec<u32>,
    pub is_ts: bool,
}

/// A 0-based LSP position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub character: u32,
}

impl FileMaps {
    /// Source position → overlay position.
    ///
    /// The token map wins where it applies: it is byte-exact and covers the
    /// synthesized regions (template interpolations, component call sites)
    /// where no line correspondence exists at all. The line map covers
    /// verbatim script blocks, where the overlay keeps the source's columns
    /// and only the line number shifts.
    pub fn to_overlay(&self, pos: Pos) -> Option<Pos> {
        let source_byte = byte_of(&self.source_line_starts, pos)?;

        if let Some(entry) = tightest(&self.token_map, source_byte, |e| {
            (e.source_byte_start, e.source_byte_end)
        }) {
            let offset = source_byte - entry.source_byte_start;
            let overlay_byte =
                (entry.overlay_byte_start + offset).min(entry.overlay_byte_end.saturating_sub(1));
            return pos_of(&self.overlay_line_starts, overlay_byte);
        }

        // Line map entries are keyed by overlay line; invert by walking them.
        let source_line = pos.line + 1;
        for e in &self.line_map {
            let span = e.overlay_end_line.saturating_sub(e.overlay_start_line);
            if source_line >= e.source_start_line && source_line <= e.source_start_line + span {
                let overlay_line = e.overlay_start_line + (source_line - e.source_start_line);
                return Some(Pos {
                    line: overlay_line.saturating_sub(1),
                    character: pos.character,
                });
            }
        }
        None
    }

    /// Overlay position → source position. The inverse of the above, and the
    /// same preference order.
    pub fn to_source(&self, pos: Pos) -> Option<Pos> {
        let overlay_byte = byte_of(&self.overlay_line_starts, pos)?;

        if let Some(entry) = tightest(&self.token_map, overlay_byte, |e| {
            (e.overlay_byte_start, e.overlay_byte_end)
        }) {
            let offset = overlay_byte - entry.overlay_byte_start;
            let source_byte =
                (entry.source_byte_start + offset).min(entry.source_byte_end.saturating_sub(1));
            return pos_of(&self.source_line_starts, source_byte);
        }

        let overlay_line = pos.line + 1;
        for e in &self.line_map {
            if overlay_line >= e.overlay_start_line && overlay_line <= e.overlay_end_line {
                let source_line = e.source_start_line + (overlay_line - e.overlay_start_line);
                return Some(Pos {
                    line: source_line.saturating_sub(1),
                    character: pos.character,
                });
            }
        }
        None
    }
}

/// The narrowest span containing `byte`. Narrowest rather than first because
/// token map entries nest — a component call site contains each of its prop
/// expressions — and the innermost one is the precise answer.
fn tightest<'a, T>(
    entries: &'a [T],
    byte: u32,
    span: impl Fn(&T) -> (u32, u32),
) -> Option<&'a T> {
    entries
        .iter()
        .filter(|e| {
            let (start, end) = span(e);
            byte >= start && byte < end
        })
        .min_by_key(|e| {
            let (start, end) = span(e);
            end - start
        })
}

fn byte_of(line_starts: &[u32], pos: Pos) -> Option<u32> {
    line_starts
        .get(pos.line as usize)
        .map(|start| start + pos.character)
}

fn pos_of(line_starts: &[u32], byte: u32) -> Option<Pos> {
    if line_starts.is_empty() {
        return None;
    }
    // Last line whose start is <= byte.
    let idx = match line_starts.binary_search(&byte) {
        Ok(i) => i,
        Err(0) => return None,
        Err(i) => i - 1,
    };
    Some(Pos {
        line: idx as u32,
        character: byte - line_starts[idx],
    })
}
