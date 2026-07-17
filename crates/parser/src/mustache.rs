//! Mustache expression parsing: find the matching `}` for an opening `{`.
//!
//! Template expressions contain arbitrary JavaScript/TypeScript, so naive
//! brace counting is insufficient — expressions can include strings,
//! template literals, regex literals, and comments that may contain unbalanced
//! braces.
//!
//! This module provides `find_mustache_end` which walks the source from just
//! after the opening `{`, tracking lexical contexts (single/double-quoted
//! strings, backtick template literals with `${}` interpolation, line and
//! block comments) and returns the byte offset of the matching `}`.
//!
//! Regex disambiguation is intentionally conservative: only a `/` in a
//! position where an expression can start is treated as a regex literal.

/// Find the byte offset of the `}` that closes the mustache block whose
/// opening `{` is at `open_brace_offset - 1`.
///
/// `expression_start` is the offset of the first char *after* the `{`.
/// Returns `None` if no matching `}` is found (truncated input).
pub fn find_mustache_end(source: &str, expression_start: u32) -> Option<u32> {
    let bytes = source.as_bytes();
    let mut i = expression_start as usize;
    let mut depth: i32 = 1;
    // Stack of template-literal `${}` context tracking. Each entry is the
    // brace depth at which the `${` was opened — when we return to that
    // depth we're back inside the template literal.
    let mut template_brace_stack: Vec<i32> = Vec::new();

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                if let Some(&top) = template_brace_stack.last() {
                    if depth == top {
                        // Returning from `${...}` into the template literal.
                        template_brace_stack.pop();
                        i += 1;
                        i = skip_template_literal(bytes, i, &mut template_brace_stack, &mut depth)?;
                        continue;
                    }
                }
                if depth == 0 {
                    return Some(i as u32);
                }
                i += 1;
            }
            b'"' => {
                i = skip_ascii_string(bytes, i + 1, b'"')?;
            }
            b'\'' => {
                i = skip_ascii_string(bytes, i + 1, b'\'')?;
            }
            b'`' => {
                i = skip_template_literal(bytes, i + 1, &mut template_brace_stack, &mut depth)?;
            }
            b'/' => {
                // Ambiguous between comment, regex, and division. Comments
                // are unambiguous; regex literals are only skipped when the
                // preceding token shape can start an expression.
                match bytes.get(i + 1).copied() {
                    Some(b'/') => {
                        // Line comment to end of line.
                        i += 2;
                        while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                            i += 1;
                        }
                    }
                    Some(b'*') => {
                        // Block comment to `*/`.
                        i += 2;
                        while i + 1 < bytes.len() {
                            if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                    }
                    _ if can_start_regex(bytes, expression_start as usize, i) => {
                        i = skip_regex_literal(bytes, i + 1)?;
                    }
                    _ => {
                        i += 1;
                    }
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    None
}

/// How a `{...}` tag dispatches. The Svelte compiler's tag lexer
/// (`phases/1-parse/state/tag.js`) advances past `{`, allows whitespace,
/// and only THEN classifies the next byte: `#` opens a block, `:`
/// continues one, `@` starts a special tag, and `/` closes a block —
/// unless it begins a `//` or `/*` comment, which belongs to an
/// expression. Everything else is an expression or declaration tag.
///
/// This is the single classifier shared by the section pre-pass, the
/// template parser's fragment dispatch, and the block-terminator reader,
/// so the three layers cannot disagree about what the same bytes mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MustacheSigil {
    /// `{#...}` — block open.
    BlockOpen,
    /// `{:...}` — block continuation (`{:else}`, `{:then}`, ...).
    Continuation,
    /// `{/...}` — block close.
    BlockClose,
    /// `{@...}` — special tag (`{@html}`, `{@const}`, ...).
    AtTag,
    /// Expression or `{const ...}`/`{let ...}` declaration tag.
    Other,
}

/// Classify the mustache tag whose `{` is at `open_brace`. Skips any
/// whitespace between the `{` and the sigil. Returns the sigil kind and
/// the byte offset of the sigil itself (for [`MustacheSigil::Other`],
/// the first non-whitespace byte — or the end of input).
pub(crate) fn classify_mustache_sigil(bytes: &[u8], open_brace: usize) -> (MustacheSigil, usize) {
    let i = crate::scanner::skip_svelte_whitespace_at(bytes, open_brace + 1);
    let kind = match bytes.get(i) {
        Some(b'#') => MustacheSigil::BlockOpen,
        Some(b':') => MustacheSigil::Continuation,
        Some(b'@') => MustacheSigil::AtTag,
        Some(b'/') if !matches!(bytes.get(i + 1), Some(b'/') | Some(b'*')) => {
            MustacheSigil::BlockClose
        }
        _ => MustacheSigil::Other,
    };
    (kind, i)
}

/// True when a slash at `slash` can begin a JS regex literal.
///
/// This is a delimiter-finding heuristic, not a full JS parser. It only opts
/// into regex mode at expression starts and after punctuators/operators where
/// division cannot validly appear, which covers Svelte block headers like
/// `{#if /re/.test(x)}` without breaking ordinary `{a / b}` expressions.
pub(crate) fn can_start_regex(bytes: &[u8], expression_start: usize, slash: usize) -> bool {
    let Some(prev) = previous_significant_index(bytes, expression_start, slash) else {
        return true;
    };
    let b = bytes[prev];
    if is_ascii_ident_continue(b) {
        return keyword_before_slash_can_start_regex(bytes, expression_start, prev);
    }
    match b {
        b'+' | b'-' => previous_significant_index(bytes, expression_start, prev)
            .is_none_or(|before| bytes[before] != b || before + 1 != prev),
        b'!' => previous_significant_index(bytes, expression_start, prev)
            .is_none_or(|before| !can_end_expression(bytes[before])),
        b'(' | b'[' | b'{' | b'=' | b':' | b',' | b';' | b'?' | b'&' | b'|' | b'*' | b'~'
        | b'^' | b'%' | b'<' | b'>' => true,
        _ => false,
    }
}

fn previous_significant_index(bytes: &[u8], expression_start: usize, end: usize) -> Option<usize> {
    let mut i = end;
    loop {
        let mut saw_newline = false;
        while i > expression_start && bytes[i - 1].is_ascii_whitespace() {
            saw_newline |= matches!(bytes[i - 1], b'\n' | b'\r');
            i -= 1;
        }
        if i <= expression_start {
            return None;
        }

        if saw_newline
            && let Some(comment_start) = line_comment_start_before(bytes, expression_start, i)
        {
            i = comment_start;
            continue;
        }

        if i >= expression_start + 2
            && bytes[i - 2] == b'*'
            && bytes[i - 1] == b'/'
            && let Some(comment_start) = block_comment_start_before(bytes, expression_start, i - 2)
        {
            i = comment_start;
            continue;
        }

        return Some(i - 1);
    }
}

fn line_comment_start_before(
    bytes: &[u8],
    expression_start: usize,
    line_end: usize,
) -> Option<usize> {
    let line_start = bytes[expression_start..line_end]
        .iter()
        .rposition(|b| matches!(b, b'\n' | b'\r'))
        .map(|offset| expression_start + offset + 1)
        .unwrap_or(expression_start);
    let mut i = line_start;
    while i + 1 < line_end {
        if bytes[i] == b'/' && bytes[i + 1] == b'/' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn block_comment_start_before(
    bytes: &[u8],
    expression_start: usize,
    block_close_start: usize,
) -> Option<usize> {
    let mut i = block_close_start;
    while i > expression_start + 1 {
        i -= 1;
        if bytes[i - 1] == b'/' && bytes[i] == b'*' {
            return Some(i - 1);
        }
    }
    None
}

fn can_end_expression(b: u8) -> bool {
    is_ascii_ident_continue(b) || matches!(b, b')' | b']' | b'}' | b'"' | b'\'' | b'`')
}

fn keyword_before_slash_can_start_regex(
    bytes: &[u8],
    expression_start: usize,
    ident_end: usize,
) -> bool {
    let mut start = ident_end;
    while start > expression_start && is_ascii_ident_continue(bytes[start - 1]) {
        start -= 1;
    }

    // `obj.return / value` is a property name followed by division, not a
    // `return` statement. Keyword interpretation only applies at a token
    // boundary that is not a member access.
    if start > expression_start {
        let before = bytes[start - 1];
        if is_ascii_ident_continue(before) || before == b'.' {
            return false;
        }
    }

    // Keywords after which `/` opens a regex, mirroring acorn's
    // `beforeExpr` keyword set (tokentype.js) plus the contextual
    // `await`/`yield` (expression-position in async/generator bodies —
    // and reserved words in module code, so they can't be identifiers
    // ending an expression). `of` is contextual-only and stays out:
    // `of / 2` must divide.
    let word = &bytes[start..=ident_end];
    matches!(
        word,
        b"return"
            | b"throw"
            | b"case"
            | b"default"
            | b"delete"
            | b"void"
            | b"typeof"
            | b"in"
            | b"instanceof"
            | b"new"
            | b"do"
            | b"else"
            | b"extends"
            | b"await"
            | b"yield"
    )
}

fn is_ascii_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$')
}

/// Skip a regex literal body after the opening slash.
///
/// Character classes are tracked separately because `/` is literal inside
/// `[...]`. Backslash escapes skip the escaped byte both inside and outside
/// character classes. Flags after the closing slash are consumed too.
pub(crate) fn skip_regex_literal(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start;
    let mut in_class = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'[' => {
                in_class = true;
                i += 1;
            }
            b']' => {
                in_class = false;
                i += 1;
            }
            b'/' if !in_class => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return Some(i);
            }
            b'\n' | b'\r' => return None,
            _ => i += 1,
        }
    }
    None
}

/// Skip past a double/single-quoted string. `start` is the byte offset of
/// the character immediately after the opening quote; `quote` is the quote
/// byte. Returns the offset of the character after the closing quote, or
/// `None` on unterminated string.
pub(crate) fn skip_ascii_string(bytes: &[u8], start: usize, quote: u8) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b if b == quote => return Some(i + 1),
            b'\\' => {
                // Escape: skip the backslash and whatever follows.
                i += 2;
            }
            b'\n' | b'\r' => {
                // Raw line terminator ends a non-template string.
                return Some(i + 1);
            }
            _ => i += 1,
        }
    }
    None
}

/// Skip past a template literal. Recognizes `${` as re-entering brace-counted
/// JS context — increments `outer_depth` and pushes the depth onto
/// `template_stack`. Returns the offset after the closing backtick, or
/// `None` on unterminated literal.
pub(crate) fn skip_template_literal(
    bytes: &[u8],
    start: usize,
    template_stack: &mut Vec<i32>,
    outer_depth: &mut i32,
) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'`' => return Some(i + 1),
            b'\\' => i += 2,
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                // Entering `${...}` — record current outer depth so we know
                // when we've exited.
                template_stack.push(*outer_depth);
                *outer_depth += 1;
                // Advance past `${` and return to the outer loop to parse
                // the JS expression inside.
                return Some(i + 2);
            }
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end_of(src: &str) -> Option<u32> {
        // src should start with `{`; we look for the matching `}`.
        assert!(src.starts_with('{'));
        find_mustache_end(src, 1)
    }

    #[test]
    fn simple_expression() {
        assert_eq!(end_of("{foo}"), Some(4));
    }

    #[test]
    fn nested_braces() {
        assert_eq!(end_of("{{ a: 1 }}"), Some(9));
        assert_eq!(end_of("{foo({ a: 1 })}"), Some(14));
    }

    #[test]
    fn string_with_brace() {
        assert_eq!(end_of("{'}'}"), Some(4));
        assert_eq!(end_of(r#"{"}"}"#), Some(4));
        assert_eq!(end_of("{\"a}b}c\"}"), Some(8));
    }

    #[test]
    fn string_with_escape() {
        assert_eq!(end_of(r#"{"a\"b}"}"#), Some(8));
    }

    #[test]
    fn template_literal_no_interpolation() {
        assert_eq!(end_of("{`foo}bar`}"), Some(10));
    }

    #[test]
    fn template_literal_with_interpolation() {
        // `${a}` inside a template literal should be accounted for.
        let src = "{`foo${a}bar`}";
        assert_eq!(end_of(src), Some(13));
    }

    #[test]
    fn template_literal_with_brace_in_interpolation() {
        let src = "{`${{a: 1}}`}";
        assert_eq!(end_of(src), Some(12));
    }

    #[test]
    fn line_comment_containing_brace() {
        let src = "{
            // }
            foo
        }";
        let end = end_of(src).expect("should find end");
        assert_eq!(&src[..end as usize + 1], src);
    }

    #[test]
    fn block_comment_containing_brace() {
        let src = "{/* } */ foo}";
        assert_eq!(end_of(src), Some(12));
    }

    #[test]
    fn regex_literal_at_expression_start() {
        let src = "{/^https?:\\/\\//.test(segment)}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn regex_literal_in_call_argument() {
        let src = "{segment.match(/[).,]+$/)?.[0] ?? ''}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn regex_literal_after_return_keyword() {
        let src = "{(() => { return /}/.test(value); })()}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn regex_literal_after_await_keyword() {
        // Async template expressions (Svelte 5.36+): `await` is
        // expression-position, so a following `/` opens a regex.
        let src = "{await /}/.test(s) ? 1 : 2}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn regex_literal_after_yield_keyword() {
        let src = "{fn(function*() { yield /}/ })}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn regex_literal_after_do_and_else_keywords() {
        let src = "{(() => { do /}/.test(s); while (0) })()}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
        let src = "{(() => { if (a) {} else /}/.test(s) })()}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn regex_literal_after_new_and_case_keywords() {
        let src = "{fn(new /}/.constructor())}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
        let src = "{(() => { switch (x) { case /}/.source: return 1; } })()}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn keyword_named_property_before_division_still_divides() {
        // `.await` / `.else` are property names — division, not regex.
        assert_eq!(end_of("{obj.await / 2}"), Some(14));
        assert_eq!(end_of("{obj.else / 2}"), Some(13));
    }

    #[test]
    fn division_after_identifier_is_not_regex_literal() {
        let src = "{a / b}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
        // `of` is no longer treated as a regex-starting keyword.
        assert_eq!(end_of("{of / 2}"), Some(7));
    }

    #[test]
    fn keyword_named_property_before_division_is_not_regex_literal() {
        let src = "{obj.return / value}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn ts_non_null_assertion_before_division_is_not_regex_literal() {
        let src = "{foo! / bar}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn postfix_increment_before_division_is_not_regex_literal() {
        let src = "{i++ / total}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn postfix_decrement_before_division_is_not_regex_literal() {
        let src = "{i-- / total}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn binary_plus_before_unary_regex_literal() {
        let src = "{foo + +/}/.source}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn binary_minus_before_unary_regex_literal() {
        let src = "{foo - -/}/.source}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn binary_plus_with_comment_before_unary_regex_literal() {
        let src = "{foo + /* comment */ +/}/.source}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn regex_literal_after_block_comment_after_return_keyword() {
        let src = "{(() => { return /* comment */ /}/.test(value); })()}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn regex_literal_after_line_comment_after_return_keyword() {
        let src = "{(() => { return // comment\n /}/.test(value); })()}";
        assert_eq!(end_of(src), Some((src.len() - 1) as u32));
    }

    #[test]
    fn unterminated_returns_none() {
        assert_eq!(end_of("{foo"), None);
        assert_eq!(end_of("{foo {bar}"), None); // second mustache not closed
    }

    #[test]
    fn empty_expression() {
        assert_eq!(end_of("{}"), Some(1));
    }

    #[test]
    fn deeply_nested_mustache_like() {
        let src = "{foo.bar({ x: { y: [1, 2] } })}";
        assert_eq!(end_of(src), Some(30));
    }
}
