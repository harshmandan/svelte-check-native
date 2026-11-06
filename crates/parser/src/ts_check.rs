//! The `// @ts-check` / `// @ts-nocheck` directives svelte2tsx copies
//! from a component's script to the top of the generated file.
//!
//! svelte2tsx takes the script (the instance script, else the module
//! script) and walks the comments its content opens with, the way
//! TypeScript's `forEachLeadingCommentRange` does: whitespace, line
//! breaks and comments of both kinds, up to the first code
//! (`extractTsCheckComment` in `svelte2tsx/nodes/directiveComment.ts`).
//! Every `//` comment whose text, after whitespace, is `@` and then the
//! name `ts-check` or `ts-nocheck` (read as lowercase letters and `-`)
//! is written out as `// @<name>`, in order, ahead of the generated
//! code. There it decides whether the compiler checks the component,
//! and when there are several, the last one wins.

/// The directives svelte2tsx copies to the top of the generated file
/// for a script with this content — `"@ts-check"` / `"@ts-nocheck"`, in
/// order.
pub fn ts_check_directives(script_content: &str) -> Vec<&'static str> {
    let mut found = Vec::new();
    for (start, end) in leading_line_comments(script_content) {
        if let Some(directive) = directive(script_content, start, end) {
            found.push(directive);
        }
    }
    found
}

/// The directive that decides whether the compiler checks a component
/// with this script content: the last of [`ts_check_directives`].
pub fn ts_check_directive(script_content: &str) -> Option<&'static str> {
    ts_check_directives(script_content).pop()
}

/// The byte ranges of the `//` comments among the leading comments of
/// `text`, each from its `//` to the line break that ends it.
fn leading_line_comments(text: &str) -> Vec<(usize, usize)> {
    let mut comments = Vec::new();
    let mut pos = 0;
    if text.starts_with("#!") {
        pos = text.find(is_line_break).unwrap_or(text.len());
    }
    while let Some(c) = text[pos..].chars().next() {
        if c == '/' && text[pos + 1..].starts_with('/') {
            let end = text[pos..]
                .find(is_line_break)
                .map_or(text.len(), |i| pos + i);
            comments.push((pos, end));
            pos = end;
        } else if c == '/' && text[pos + 1..].starts_with('*') {
            pos = text[pos + 2..]
                .find("*/")
                .map_or(text.len(), |i| pos + 2 + i + 2);
        } else if is_white_space_like(c) {
            pos += c.len_utf8();
        } else {
            break;
        }
    }
    comments
}

/// The directive the `//` comment at `start..end` of `text` names, if
/// any. As in svelte2tsx, the whitespace skipped after `//` is not
/// bounded by the comment, while the name is.
fn directive(text: &str, start: usize, end: usize) -> Option<&'static str> {
    let after = text[start + 2..].trim_start_matches(is_white_space_like);
    let name_start = text.len() - after.len() + 1;
    if !after.starts_with('@') || name_start > end {
        return None;
    }
    let name_len = text[name_start..end]
        .find(|c: char| !(c.is_ascii_lowercase() || c == '-'))
        .unwrap_or(end - name_start);
    match &text[name_start..name_start + name_len] {
        "ts-check" => Some("@ts-check"),
        "ts-nocheck" => Some("@ts-nocheck"),
        _ => None,
    }
}

/// TypeScript's `isWhiteSpaceLike`: single-line whitespace or a line
/// break.
fn is_white_space_like(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200B}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    ) || is_line_break(c)
}

/// TypeScript's `isLineBreak`.
fn is_line_break(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_directive_in_the_leading_line_comments() {
        assert_eq!(ts_check_directives("\n// @ts-check\nlet x;"), ["@ts-check"]);
        assert_eq!(
            ts_check_directives("// hi\n//\t@ts-nocheck\nlet x;"),
            ["@ts-nocheck"]
        );
        assert_eq!(
            ts_check_directives("// @ts-check\n// @ts-nocheck\n"),
            ["@ts-check", "@ts-nocheck"]
        );
        // A block comment is a leading comment too, and doesn't count.
        assert_eq!(
            ts_check_directives("/* a */\n// @ts-check\n"),
            ["@ts-check"]
        );
        assert!(ts_check_directives("/* @ts-check */\n").is_empty());
        // The name stops at anything but a lowercase letter or `-`.
        assert_eq!(ts_check_directives("// @ts-check: on\n"), ["@ts-check"]);
        assert!(ts_check_directives("// @ts-checked\n").is_empty());
        assert!(ts_check_directives("// @TS-check\n").is_empty());
        // Only comments before any code.
        assert!(ts_check_directives("let x;\n// @ts-check\n").is_empty());
    }

    #[test]
    fn svelte2tsx_ts_check_sample() {
        let content = "\n    // @ts-check dsaafd\n    // @ts-nocheck dsafs\n    // @dafts-check\n    \
                       // @ts-checkabc\n    //@ts-check\n    //    @ts-check\n";
        assert_eq!(
            ts_check_directives(content),
            ["@ts-check", "@ts-nocheck", "@ts-check", "@ts-check"]
        );
    }

    #[test]
    fn the_last_directive_decides() {
        assert_eq!(
            ts_check_directive("// @ts-nocheck\n// @ts-check\n"),
            Some("@ts-check")
        );
        assert_eq!(ts_check_directive("let x;"), None);
    }
}
