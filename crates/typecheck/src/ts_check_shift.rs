//! Position skew svelte-check applies to components whose script opens
//! with a `// @ts-check` or `// @ts-nocheck` comment.
//!
//! svelte-check maps a tsgo diagnostic back to the `.svelte` source
//! through a document snapshot it builds by re-running svelte2tsx. When
//! the script (the instance script, else the module script) starts with
//! line comments and one of them is a `@ts-check` / `@ts-nocheck`
//! directive, that snapshot's text is the generated code with the
//! directive line (`// @ts-check\n`) prepended, and its mapper subtracts
//! one line before consulting the source map. The diagnostic offset,
//! however, was measured in the file tsgo actually checked, which has no
//! such prefix. Reading that offset against the prefixed text and then
//! dropping a line lands the diagnostic exactly `prefix length` UTF-16
//! units earlier in the generated code than where tsgo put it, so every
//! diagnostic in such a component is reported that far before its real
//! position. Parity means applying the same skew to our overlay position
//! before mapping it.

use crate::position::{byte_to_position, position_to_byte};
use crate::types::MapData;

/// Length, in UTF-16 units, of the directive line svelte-check prepends
/// for this component, or 0 when it prepends nothing.
pub(crate) fn prefix_len(source: &str) -> u32 {
    let (doc, _) = svn_parser::parse_sections(source);
    let Some(script) = doc.instance_script.as_ref().or(doc.module_script.as_ref()) else {
        return 0;
    };
    match svn_parser::ts_check_directive(script.content) {
        // `// ` + directive + the platform newline, which is `\n` on
        // every platform svelte-check writes these paths from except
        // Windows.
        Some(d) => (3 + d.len() + NEWLINE.len()) as u32,
        None => 0,
    }
}

#[cfg(windows)]
const NEWLINE: &str = "\r\n";
#[cfg(not(windows))]
const NEWLINE: &str = "\n";

/// Move a 1-based overlay `(line, column)` back by `data.ts_check_prefix`
/// UTF-16 units, clamping at the start of the file.
pub(crate) fn skew(data: &MapData, line: u32, column: u32) -> (u32, u32) {
    let text = data.overlay_text.get();
    let Some(byte) = position_to_byte(&data.overlay_line_starts, text, line, column) else {
        return (line, column);
    };
    let mut remaining = data.ts_check_prefix;
    let mut at = byte as usize;
    let head = text.get(..at).unwrap_or("");
    for ch in head.chars().rev() {
        if remaining == 0 {
            break;
        }
        remaining = remaining.saturating_sub(ch.len_utf16() as u32);
        at -= ch.len_utf8();
    }
    byte_to_position(&data.overlay_line_starts, text, at as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_follows_the_leading_comment_run() {
        assert_eq!(prefix_len("<script>\n// @ts-check\nlet x;</script>"), 13);
        assert_eq!(
            prefix_len("<script>\n// hi\n//\t@ts-nocheck\nlet x;</script>"),
            15
        );
        // The directive must be a whole word.
        assert_eq!(prefix_len("<script>// @ts-checked\n</script>"), 0);
        // Only comments before any code count.
        assert_eq!(prefix_len("<script>let x;\n// @ts-check\n</script>"), 0);
        // A block comment ends the run.
        assert_eq!(prefix_len("<script>/* a */\n// @ts-check\n</script>"), 0);
        // The instance script wins over the module script.
        assert_eq!(
            prefix_len("<script module>// @ts-check\n</script><script>let x;</script>"),
            0
        );
        assert_eq!(prefix_len("<script module>// @ts-check\n</script>"), 13);
        assert_eq!(prefix_len("<div></div>"), 0);
    }

    #[test]
    fn skew_walks_back_across_lines() {
        let text = "ab\n// @ts-check\nlet x = 1;\nx.foo();\n";
        let data = MapData {
            overlay_line_starts: svn_emit::compute_line_starts(text),
            overlay_text: text.into(),
            ts_check_prefix: 13,
            ..Default::default()
        };
        // `foo` (line 4, column 3) lands on the start of `let`.
        assert_eq!(skew(&data, 4, 3), (3, 1));
        // Clamps at the start of the file.
        assert_eq!(skew(&data, 1, 2), (1, 1));
    }
}
