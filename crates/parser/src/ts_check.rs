//! The `// @ts-check` / `// @ts-nocheck` directive svelte-check reads
//! from a component's script.
//!
//! svelte-check takes the script (the instance script, else the module
//! script), keeps the run of `//` comments it opens with, and looks for
//! the first `@ts-check` / `@ts-nocheck` among them
//! (`getTsCheckComment` in the language server's
//! `plugins/typescript/utils.ts`). The directive it finds is prepended
//! to the generated file, so it decides whether the compiler checks the
//! component at all.

/// The directive the script content opens with — `"@ts-check"` or
/// `"@ts-nocheck"` — or `None`.
pub fn ts_check_directive(script_content: &str) -> Option<&'static str> {
    directive(leading_line_comments(script_content))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_directive_in_the_leading_line_comments() {
        assert_eq!(
            ts_check_directive("\n// @ts-check\nlet x;"),
            Some("@ts-check")
        );
        assert_eq!(
            ts_check_directive("// hi\n//\t@ts-nocheck\nlet x;"),
            Some("@ts-nocheck")
        );
        assert_eq!(
            ts_check_directive("// @ts-check\n// @ts-nocheck\n"),
            Some("@ts-check")
        );
        // A whole word, before any code, in line comments only.
        assert_eq!(ts_check_directive("// @ts-checked\n"), None);
        assert_eq!(ts_check_directive("let x;\n// @ts-check\n"), None);
        assert_eq!(ts_check_directive("/* a */\n// @ts-check\n"), None);
    }
}
