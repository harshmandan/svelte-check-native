//! Text-content rules (fire on template Text nodes).

use svn_core::Range;
use svn_parser::ast::{Node, Text};

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

/// `preceding` holds the siblings before `t` in its fragment.
pub fn visit_text(t: &Text, preceding: &[&Node], ctx: &mut LintContext<'_>) {
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
    for run in bidi_runs(t.range, ctx.source) {
        // A text node does not take the ignore comments before it
        // the way elements do. Instead, each match looks at every
        // earlier comment in the fragment (not just the adjacent
        // ones) and stops at the first that ignores the warning;
        // each comment it parses reports its unknown codes again.
        let mut is_ignored = false;
        for sibling in preceding {
            if is_ignored {
                break;
            }
            if let Node::Comment(c) = *sibling {
                is_ignored = crate::ignore::comment_ignores_code(
                    c,
                    Code::bidirectional_control_characters,
                    ctx,
                );
            }
        }
        if !is_ignored {
            let msg = messages::bidirectional_control_characters();
            ctx.emit(Code::bidirectional_control_characters, msg, run);
        }
    }
}

/// The text chunks of an attribute value are text nodes to the
/// compiler too; outside a fragment they skip the comment scan.
pub fn visit_attribute_text(range: Range, ctx: &mut LintContext<'_>) {
    for run in bidi_runs(range, ctx.source) {
        let msg = messages::bidirectional_control_characters();
        ctx.emit(Code::bidirectional_control_characters, msg, run);
    }
}

/// Each maximal run of bidi-control characters in `range`.
fn bidi_runs(range: Range, source: &str) -> Vec<Range> {
    let base = range.start;
    let content = range.slice(source);
    let mut runs = Vec::new();
    let mut chars = content.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if is_bidi_control(c) {
            let mut end = i + c.len_utf8();
            while let Some(&(j, nc)) = chars.peek() {
                if !is_bidi_control(nc) {
                    break;
                }
                chars.next();
                end = j + nc.len_utf8();
            }
            runs.push(Range::new(base + i as u32, base + end as u32));
        }
    }
    runs
}
