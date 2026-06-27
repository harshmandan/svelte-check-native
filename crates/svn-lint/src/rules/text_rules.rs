//! Text-content rules (fire on template Text nodes).

use svn_core::Range;
use svn_parser::ast::Text;

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

/// Bidirectional-override control characters. Matches upstream
/// `patterns.js::regex_bidirectional_control_characters` —
/// `/[\u202a-\u202e\u2066-\u2069]+/g` (the `+` is honored by the
/// contiguous-run merge in `visit_text`, one warning per maximal run).
fn is_bidi_control(c: char) -> bool {
    matches!(
        c as u32,
        0x202A..=0x202E | 0x2066..=0x2069
    )
}

pub fn visit_text(t: &Text, ctx: &mut LintContext<'_>) {
    // The `#text` `node_invalid_placement` ERROR (e.g. raw text where
    // the HTML5 tree model forbids it) is intentionally not emitted in
    // native mode — same stance as the element placement path. Native
    // lint produces the warning surface only; the hard placement error
    // is delivered by the `svelte/compiler` bridge under
    // `--svelte-warnings=bridge`. Implementing the error here for
    // `#text` alone would diverge from the element path and would change
    // default-mode output, so it stays out of the native pass.
    //
    // Walk contiguous runs of bidi-control chars — upstream fires
    // once per match (each match is a run of 1+ of these chars).
    let start_byte = t.range.start as usize;
    let content = &t.content;
    let mut chars = content.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if is_bidi_control(c) {
            // Expand to contiguous run.
            let run_start = i;
            let mut run_end = i + c.len_utf8();
            while let Some(&(j, nc)) = chars.peek() {
                if !is_bidi_control(nc) {
                    break;
                }
                chars.next();
                run_end = j + nc.len_utf8();
            }
            let abs_start = (start_byte + run_start) as u32;
            let abs_end = (start_byte + run_end) as u32;
            let msg = messages::bidirectional_control_characters();
            ctx.emit(
                Code::bidirectional_control_characters,
                msg,
                Range::new(abs_start, abs_end),
            );
        }
    }
}
