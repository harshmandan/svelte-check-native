//! Text-content rules (fire on template Text nodes).

use svn_core::Range;
use svn_parser::ast::Text;

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

/// Bidirectional-override control characters. Matches upstream
/// `patterns.js::regex_bidirectional_control_characters` —
/// `/[\u202a-\u202e\u2066-\u2069]/g`.
fn is_bidi_control(c: char) -> bool {
    matches!(
        c as u32,
        0x202A..=0x202E | 0x2066..=0x2069
    )
}

pub fn visit_text(t: &Text, ctx: &mut LintContext<'_>) {
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
