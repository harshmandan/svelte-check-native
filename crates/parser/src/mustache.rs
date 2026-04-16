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
//! Regex disambiguation is *not* attempted — a `/` after an operator or at
//! statement start is ambiguous with division, and correctly resolving it
//! requires full expression context. In practice regex literals are
//! uncommon in template mustaches; when they cause failures we can revisit.

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
                // Ambiguous between comment, regex, and division. We handle
                // the two unambiguous comment forms and otherwise advance by
                // one.
                match bytes.get(i + 1).copied() {
                    Some(b'/') => {
                        // Line comment to end of line.
                        i += 2;
                        while i < bytes.len() && bytes[i] != b'\n' {
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

/// Skip past a double/single-quoted string. `start` is the byte offset of
/// the character immediately after the opening quote; `quote` is the quote
/// byte. Returns the offset of the character after the closing quote, or
/// `None` on unterminated string.
fn skip_ascii_string(bytes: &[u8], start: usize, quote: u8) -> Option<usize> {
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b if b == quote => return Some(i + 1),
            b'\\' => {
                // Escape: skip the backslash and whatever follows.
                i += 2;
            }
            b'\n' => {
                // Raw newline ends a non-template string. Treat as terminated
                // (caller will see the real syntax error from oxc later).
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
fn skip_template_literal(
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
