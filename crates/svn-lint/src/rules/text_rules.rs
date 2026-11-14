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
    for run in bidi_runs(t.range, ctx.source, false) {
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
    for run in bidi_runs(range, ctx.source, true) {
        let msg = messages::bidirectional_control_characters();
        ctx.emit(Code::bidirectional_control_characters, msg, run);
    }
}

/// Each maximal run of bidi-control characters in the text at `range`.
///
/// The compiler looks for them in the text with its character
/// references decoded, and reports a run at its place in that decoded
/// text counted from where the node starts in the file. A run written
/// as `&#x202E;` is therefore reported one character wide, at the
/// offset it has once the references before it are decoded.
fn bidi_runs(range: Range, source: &str, is_attribute_value: bool) -> Vec<Range> {
    let content = range.slice(source);
    let decoded = crate::html_decode::decode_character_references(content, is_attribute_value);
    // Where the character `units` UTF-16 units after the node's start
    // begins in the file.
    let offset_after = |units: usize| {
        let mut left = units;
        let rest = source.get(range.start as usize..).unwrap_or("");
        for (i, c) in rest.char_indices() {
            if left == 0 {
                return range.start + i as u32;
            }
            left = left.saturating_sub(c.len_utf16());
        }
        source.len() as u32
    };
    let mut runs = Vec::new();
    let mut units = 0;
    let mut run_start: Option<usize> = None;
    for c in decoded.chars() {
        if is_bidi_control(c) {
            run_start.get_or_insert(units);
        } else if let Some(start) = run_start.take() {
            runs.push(Range::new(offset_after(start), offset_after(units)));
        }
        units += c.len_utf16();
    }
    if let Some(start) = run_start {
        runs.push(Range::new(offset_after(start), offset_after(units)));
    }
    runs
}
