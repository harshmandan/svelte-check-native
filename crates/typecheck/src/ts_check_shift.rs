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
    match directive(leading_line_comments(script.content)) {
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

/// The run of `//` comments (and the whitespace around them) the script
/// opens with — `/^(\s*\/\/.*\s*)*/` in svelte-check's terms, where `.`
/// stops at a line break.
fn leading_line_comments(content: &str) -> &str {
    let mut end = 0;
    loop {
        let rest = &content[end..];
        let after_ws = rest.trim_start_matches(is_js_whitespace);
        if !after_ws.starts_with("//") {
            return &content[..end];
        }
        let comment_start = content.len() - after_ws.len();
        let line_end = after_ws
            .find(is_line_terminator)
            .map_or(content.len(), |i| comment_start + i);
        let after = content[line_end..].trim_start_matches(is_js_whitespace);
        end = content.len() - after.len();
    }
}

/// First `//` comment in `comments` whose text, after optional
/// non-breaking whitespace, is `@ts-check` or `@ts-nocheck` followed by
/// whitespace or the end of `comments`.
fn directive(comments: &str) -> Option<&'static str> {
    let mut search = comments;
    while let Some(i) = search.find("//") {
        let after = search[i + 2..].trim_start_matches(|c: char| {
            matches!(
                c,
                ' ' | '\t' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
                    ..='\u{200A}'
                        | '\u{2028}'
                        | '\u{2029}'
                        | '\u{202F}'
                        | '\u{205F}'
                        | '\u{3000}'
                        | '\u{FEFF}'
            )
        });
        for d in ["@ts-check", "@ts-nocheck"] {
            if let Some(tail) = after.strip_prefix(d)
                && tail.chars().next().is_none_or(is_js_whitespace)
            {
                return Some(d);
            }
        }
        search = &search[i + 2..];
    }
    None
}

/// JavaScript's `\s` class.
fn is_js_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}'
}

/// What a regex `.` refuses to match.
fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

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
