//! Control-flow block parsing.
//!
//! Handles `{#if}`, `{#each}`, `{#await}`, `{#key}`, `{#snippet}` and their
//! branch tags (`{:else}`, `{:else if}`, `{:then}`, `{:catch}`). Called from
//! the main template parser when it encounters `{#...}`.
//!
//! Each block has a header (`{#kind ...}`) with parameters specific to that
//! block, one or more body fragments separated by branch tags, and a close
//! tag (`{/kind}`).
//!
//! Branch-tag handling lives in the main template parser because a child
//! fragment's termination condition is "until we hit the next `{:`
//! branch-tag or `{/kind}` close". The child parser scans for those; when it
//! sees one, it returns control here to dispatch the next branch.

use smol_str::SmolStr;
use svn_core::Range;

use crate::ast::{
    AwaitBlock, CatchBranch, EachAsClause, EachBlock, ElseIfArm, Fragment, IfBlock, KeyBlock,
    SnippetBlock, ThenBranch,
};
use crate::error::ParseError;
use crate::mustache::find_mustache_end;
use crate::scanner::Scanner;

/// What block terminator was found in a child fragment scan.
#[derive(Debug)]
pub(crate) enum BlockTerminator {
    /// `{/if}`, `{/each}`, `{/await}`, `{/key}`, `{/snippet}`. `tag` is the
    /// block name without `/`. Scanner positioned just past the terminator.
    Close { tag: String },
    /// `{:else}` — no trailing condition.
    Else,
    /// `{:else if condition}` — trailing expression range captured.
    ElseIf { condition_range: Range },
    /// `{:then [binding]}` — optional binding context range.
    Then { context_range: Option<Range> },
    /// `{:catch [binding]}` — optional binding context range.
    Catch { context_range: Option<Range> },
}

/// Try to recognize a branch tag (`{:...}`) or close tag (`{/...}`) at the
/// scanner's position. Returns the terminator and advances past it on
/// success; leaves the scanner unchanged on non-match.
pub(crate) fn peek_and_consume_terminator(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
) -> Option<BlockTerminator> {
    let start = scanner.pos();
    if !scanner.starts_with("{:") && !scanner.starts_with("{/") {
        return None;
    }
    let is_close = scanner.starts_with("{/");
    scanner.advance(2); // past `{:` or `{/`

    // Read tag word.
    let word_start = scanner.pos();
    while let Some(b) = scanner.peek_byte() {
        if b.is_ascii_alphabetic() {
            scanner.advance_byte();
        } else {
            break;
        }
    }
    let word_end = scanner.pos();
    let word = &scanner.source()[word_start as usize..word_end as usize];

    if is_close {
        // `{/tag}` — skip whitespace and expect `}`.
        scanner.skip_ascii_whitespace();
        if scanner.peek_byte() != Some(b'}') {
            errors.push(ParseError::MalformedOpenTag {
                range: Range::new(start, scanner.pos()),
            });
            scanner.set_pos(start);
            return None;
        }
        scanner.advance_byte();
        return Some(BlockTerminator::Close {
            tag: word.to_string(),
        });
    }

    // Branch tag.
    match word {
        "else" => {
            scanner.skip_ascii_whitespace();
            if scanner.peek_byte() == Some(b'}') {
                scanner.advance_byte();
                Some(BlockTerminator::Else)
            } else if scanner.starts_with("if") {
                scanner.advance(2);
                scanner.skip_ascii_whitespace();
                let cond_start = scanner.pos();
                let end = find_matching_close_brace(scanner)?;
                let condition_range = Range::new(cond_start, end);
                scanner.set_pos(end + 1);
                Some(BlockTerminator::ElseIf { condition_range })
            } else {
                errors.push(ParseError::MalformedOpenTag {
                    range: Range::new(start, scanner.pos()),
                });
                scanner.set_pos(start);
                None
            }
        }
        "then" | "catch" => {
            scanner.skip_ascii_whitespace();
            // Optional binding before `}`.
            let ctx_start = scanner.pos();
            // Read until `}`.
            while let Some(b) = scanner.peek_byte() {
                if b == b'}' {
                    break;
                }
                scanner.advance_char();
            }
            let ctx_end = scanner.pos();
            if scanner.peek_byte() != Some(b'}') {
                errors.push(ParseError::MalformedOpenTag {
                    range: Range::new(start, scanner.pos()),
                });
                scanner.set_pos(start);
                return None;
            }
            scanner.advance_byte();
            let ctx_trimmed_start = skip_ws_start(scanner.source(), ctx_start, ctx_end);
            let ctx_trimmed_end = skip_ws_end(scanner.source(), ctx_trimmed_start, ctx_end);
            let context_range = if ctx_trimmed_start < ctx_trimmed_end {
                Some(Range::new(ctx_trimmed_start, ctx_trimmed_end))
            } else {
                None
            };
            if word == "then" {
                Some(BlockTerminator::Then { context_range })
            } else {
                Some(BlockTerminator::Catch { context_range })
            }
        }
        _ => {
            errors.push(ParseError::MalformedOpenTag {
                range: Range::new(start, scanner.pos()),
            });
            scanner.set_pos(start);
            None
        }
    }
}

/// Find the matching `}` from the current scanner position (which must be
/// on the expression start, i.e. just after `{...` prefix). Does not
/// advance the scanner.
fn find_matching_close_brace(scanner: &Scanner<'_>) -> Option<u32> {
    find_mustache_end(scanner.source(), scanner.pos())
}

fn skip_ws_start(src: &str, mut start: u32, end: u32) -> u32 {
    let bytes = src.as_bytes();
    while start < end && bytes[start as usize].is_ascii_whitespace() {
        start += 1;
    }
    start
}

fn skip_ws_end(src: &str, start: u32, mut end: u32) -> u32 {
    let bytes = src.as_bytes();
    while end > start && bytes[(end - 1) as usize].is_ascii_whitespace() {
        end -= 1;
    }
    end
}

// ===== Header parsing ====================================================

/// Header for an `{#if expression}` block. Scanner must be positioned right
/// after `{#if` when called. On return, scanner is just past the closing `}`
/// of the header.
pub(crate) fn parse_if_header(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
) -> Option<Range> {
    scanner.skip_ascii_whitespace();
    let start = scanner.pos();
    let end = find_matching_close_brace(scanner)?;
    if end <= start {
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(start, end.max(start)),
        });
        return None;
    }
    scanner.set_pos(end + 1);
    Some(Range::new(start, end))
}

/// Parse the header of `{#each expression [as pattern[, index]] [(key)]}`.
///
/// Returns `(expression_range, Option<EachAsClause>)`. The scanner advances
/// past the closing `}`. Because `as` / `,` / `(` / `)` can appear inside
/// expressions, we use the same lexical context tracker as for top-level
/// commas.
pub(crate) fn parse_each_header(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
) -> Option<(Range, Option<EachAsClause>)> {
    scanner.skip_ascii_whitespace();
    let header_start = scanner.pos();
    let header_end = find_matching_close_brace(scanner)?;
    if header_end <= header_start {
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(header_start, header_end.max(header_start)),
        });
        return None;
    }

    let header = &scanner.source()[header_start as usize..header_end as usize];
    // Find top-level ` as ` token.
    let as_pos = find_token_at_depth_zero(header, " as ");

    let (expression_range, as_clause) = match as_pos {
        Some(p) => {
            let expr_end = header_start + p as u32;
            let clause_start = expr_end + 4; // past " as "
            let clause_str = &scanner.source()[clause_start as usize..header_end as usize];
            let clause = parse_each_as_clause(clause_str, clause_start);
            (Range::new(header_start, expr_end), Some(clause))
        }
        None => (Range::new(header_start, header_end), None),
    };

    scanner.set_pos(header_end + 1);
    Some((expression_range, as_clause))
}

fn parse_each_as_clause(clause: &str, base_offset: u32) -> EachAsClause {
    // `clause` may look like:
    //   "item"
    //   "item, i"
    //   "item, i (key)"
    //   "{ a, b }"
    //   "_, i"
    // We locate the top-level `(` (if any) and `,` (splits context from index).

    let bytes = clause.as_bytes();

    // Find `(` at depth 0 to split context+index from the key expression.
    let paren_pos = find_char_at_depth_zero(clause, b'(');

    let (before_key, key_range) = if let Some(p) = paren_pos {
        // Key expression is everything up to matching `)`.
        let key_inner_start = p + 1;
        let mut depth = 1;
        let mut i = key_inner_start;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        let key_end = i;
        let key_start_trim = skip_ws_start(clause, key_inner_start as u32, key_end as u32);
        let key_end_trim = skip_ws_end(clause, key_start_trim, key_end as u32);
        let key_range = if key_start_trim < key_end_trim {
            Some(Range::new(
                base_offset + key_start_trim,
                base_offset + key_end_trim,
            ))
        } else {
            None
        };
        (&clause[..p], key_range)
    } else {
        (clause, None)
    };

    // Split before_key on depth-0 comma into context + optional index.
    let comma_pos = find_char_at_depth_zero(before_key, b',');

    let (context_slice, index_slice) = match comma_pos {
        Some(p) => (&before_key[..p], Some(&before_key[p + 1..])),
        None => (before_key, None),
    };

    let context_start = skip_ws_start(clause, 0, context_slice.len() as u32);
    let context_end = skip_ws_end(clause, context_start, context_slice.len() as u32);
    let context_range = Range::new(base_offset + context_start, base_offset + context_end);

    let index_range = match (comma_pos, index_slice) {
        (Some(comma), Some(idx)) => {
            let index_offset = comma + 1;
            let idx_end = context_slice.len() + 1 + idx.len();
            let idx_start_trim = skip_ws_start(clause, index_offset as u32, idx_end as u32);
            let idx_end_trim = skip_ws_end(clause, idx_start_trim, idx_end as u32);
            Some(Range::new(
                base_offset + idx_start_trim,
                base_offset + idx_end_trim,
            ))
        }
        _ => None,
    };

    EachAsClause {
        context_range,
        index_range,
        key_range,
    }
}

/// Parse `{#await expression [then pattern | catch pattern]}` header.
///
/// Returns `(expression_range, short_form)` where `short_form` is one of
/// `None` (standard block, will use `{:then}` and `{:catch}` branches),
/// `Some((then_context, None))` for `{#await p then v}`, or
/// `Some((None, catch_context))` for `{#await p catch e}`.
pub(crate) fn parse_await_header(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
) -> Option<(Range, AwaitShortForm)> {
    scanner.skip_ascii_whitespace();
    let header_start = scanner.pos();
    let header_end = find_matching_close_brace(scanner)?;
    if header_end <= header_start {
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(header_start, header_end.max(header_start)),
        });
        return None;
    }

    let header = &scanner.source()[header_start as usize..header_end as usize];

    // Look for `then` or `catch` separator at depth zero. Either form may
    // be followed by an optional context binding before the closing `}`.
    let (short, split_at) = if let Some(p) = find_token_at_depth_zero(header, " then ") {
        let ctx_start = header_start + (p + 6) as u32;
        let ctx = trimmed_range(scanner.source(), ctx_start, header_end);
        (AwaitShortForm::Then(ctx), Some(p))
    } else if let Some(p) = find_token_at_depth_zero(header, " catch ") {
        let ctx_start = header_start + (p + 7) as u32;
        let ctx = trimmed_range(scanner.source(), ctx_start, header_end);
        (AwaitShortForm::Catch(ctx), Some(p))
    } else if let Some(p) = header
        .strip_suffix(" then")
        .map(|h| h.len())
        .or_else(|| if header == "then" { Some(0) } else { None })
    {
        (AwaitShortForm::Then(None), Some(p))
    } else if let Some(p) = header
        .strip_suffix(" catch")
        .map(|h| h.len())
        .or_else(|| if header == "catch" { Some(0) } else { None })
    {
        (AwaitShortForm::Catch(None), Some(p))
    } else {
        (AwaitShortForm::None, None)
    };

    let expression_range = match split_at {
        Some(p) => Range::new(header_start, header_start + p as u32),
        None => Range::new(header_start, header_end),
    };

    scanner.set_pos(header_end + 1);
    Some((expression_range, short))
}

#[derive(Debug)]
pub(crate) enum AwaitShortForm {
    None,
    Then(Option<Range>),
    Catch(Option<Range>),
}

/// `{#key expression}` header.
pub(crate) fn parse_key_header(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
) -> Option<Range> {
    parse_if_header(scanner, errors)
}

/// `{#snippet name(params)}` header.
///
/// Returns `(name, params_range)` where `params_range` is the byte range of
/// the parameter list (inside the parens, not including them).
pub(crate) fn parse_snippet_header(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
) -> Option<(SmolStr, Range)> {
    scanner.skip_ascii_whitespace();

    let name_start = scanner.pos();
    while let Some(b) = scanner.peek_byte() {
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
            scanner.advance_byte();
        } else {
            break;
        }
    }
    let name_end = scanner.pos();
    if name_end == name_start {
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(name_start, name_end.max(name_start + 1)),
        });
        return None;
    }
    let name: SmolStr = scanner.source()[name_start as usize..name_end as usize].into();

    scanner.skip_ascii_whitespace();
    if scanner.peek_byte() != Some(b'(') {
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(scanner.pos(), scanner.pos()),
        });
        return None;
    }
    scanner.advance_byte();
    let params_start = scanner.pos();

    // Find matching `)`.
    let mut depth = 1;
    while !scanner.eof() {
        match scanner.peek_byte() {
            Some(b'(') => depth += 1,
            Some(b')') => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        scanner.advance_char();
    }
    let params_end = scanner.pos();
    if scanner.peek_byte() != Some(b')') {
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(params_start, params_end),
        });
        return None;
    }
    scanner.advance_byte(); // past `)`
    scanner.skip_ascii_whitespace();
    if scanner.peek_byte() != Some(b'}') {
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(scanner.pos(), scanner.pos()),
        });
        return None;
    }
    scanner.advance_byte();

    Some((name, Range::new(params_start, params_end)))
}

// ===== Lexical helpers ===================================================

/// Find a token at depth 0 (outside strings / template literals / nested
/// parens / brackets / braces / comments). Searches for `needle` as a
/// contiguous byte match.
fn find_token_at_depth_zero(src: &str, needle: &str) -> Option<usize> {
    let bytes = src.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut i = 0;
    let mut depth: i32 = 0;

    while i < bytes.len() {
        if depth == 0
            && i + needle_bytes.len() <= bytes.len()
            && &bytes[i..i + needle_bytes.len()] == needle_bytes
        {
            return Some(i);
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'"' | b'\'' => {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'`' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'`' {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn find_char_at_depth_zero(src: &str, ch: u8) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut depth: i32 = 0;
    while i < bytes.len() {
        if depth == 0 && bytes[i] == ch {
            return Some(i);
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'"' | b'\'' => {
                let q = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != q {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            b'`' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'`' {
                    if bytes[i] == b'\\' {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn trimmed_range(src: &str, start: u32, end: u32) -> Option<Range> {
    let s = skip_ws_start(src, start, end);
    let e = skip_ws_end(src, s, end);
    if s < e { Some(Range::new(s, e)) } else { None }
}

// ===== Assembly helpers (for the template parser) ======================

/// Assemble an IfBlock from its consequent + else-if arms + alternate.
pub(crate) fn build_if_block(
    condition_range: Range,
    consequent: Fragment,
    elseif_arms: Vec<ElseIfArm>,
    alternate: Option<Fragment>,
    block_start: u32,
    block_end: u32,
) -> IfBlock {
    IfBlock {
        condition_range,
        consequent,
        elseif_arms,
        alternate,
        range: Range::new(block_start, block_end),
    }
}

/// Assemble an EachBlock.
pub(crate) fn build_each_block(
    expression_range: Range,
    as_clause: Option<EachAsClause>,
    body: Fragment,
    alternate: Option<Fragment>,
    block_start: u32,
    block_end: u32,
) -> EachBlock {
    EachBlock {
        expression_range,
        as_clause,
        body,
        alternate,
        range: Range::new(block_start, block_end),
    }
}

/// Assemble an AwaitBlock.
pub(crate) fn build_await_block(
    expression_range: Range,
    pending: Option<Fragment>,
    then_branch: Option<ThenBranch>,
    catch_branch: Option<CatchBranch>,
    block_start: u32,
    block_end: u32,
) -> AwaitBlock {
    AwaitBlock {
        expression_range,
        pending,
        then_branch,
        catch_branch,
        range: Range::new(block_start, block_end),
    }
}

/// Assemble a KeyBlock.
pub(crate) fn build_key_block(
    expression_range: Range,
    body: Fragment,
    block_start: u32,
    block_end: u32,
) -> KeyBlock {
    KeyBlock {
        expression_range,
        body,
        range: Range::new(block_start, block_end),
    }
}

/// Assemble a SnippetBlock.
pub(crate) fn build_snippet_block(
    name: SmolStr,
    parameters_range: Range,
    body: Fragment,
    block_start: u32,
    block_end: u32,
) -> SnippetBlock {
    SnippetBlock {
        name,
        parameters_range,
        body,
        range: Range::new(block_start, block_end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_token_at_depth_zero_basic() {
        assert_eq!(find_token_at_depth_zero("foo as bar", " as "), Some(3));
        // Same token inside parens: depth != 0, skip.
        assert_eq!(
            find_token_at_depth_zero("foo(a as b) as c", " as "),
            Some(11)
        );
    }

    #[test]
    fn find_char_at_depth_zero_basic() {
        assert_eq!(find_char_at_depth_zero("a,b", b','), Some(1));
        assert_eq!(find_char_at_depth_zero("f(a, b),c", b','), Some(7));
    }

    #[test]
    fn find_char_at_depth_zero_in_string_skipped() {
        assert_eq!(find_char_at_depth_zero(r#""a,b",c"#, b','), Some(5));
    }

    #[test]
    fn parse_each_as_clause_simple_identifier() {
        let clause = "item";
        let c = parse_each_as_clause(clause, 10);
        assert_eq!(c.context_range, Range::new(10, 14));
        assert!(c.index_range.is_none());
        assert!(c.key_range.is_none());
    }

    #[test]
    fn parse_each_as_clause_with_index() {
        let clause = "item, i";
        let c = parse_each_as_clause(clause, 0);
        assert_eq!(c.context_range.slice(clause), "item");
        assert_eq!(c.index_range.map(|r| r.slice(clause)), Some("i"));
    }

    #[test]
    fn parse_each_as_clause_with_key() {
        let clause = "item (item.id)";
        let c = parse_each_as_clause(clause, 0);
        assert_eq!(c.context_range.slice(clause), "item");
        assert_eq!(c.key_range.map(|r| r.slice(clause)), Some("item.id"));
    }

    #[test]
    fn parse_each_as_clause_full() {
        let clause = "item, i (item.id)";
        let c = parse_each_as_clause(clause, 0);
        assert_eq!(c.context_range.slice(clause), "item");
        assert_eq!(c.index_range.map(|r| r.slice(clause)), Some("i"));
        assert_eq!(c.key_range.map(|r| r.slice(clause)), Some("item.id"));
    }

    #[test]
    fn parse_each_as_clause_destructuring() {
        let clause = "{ a, b }, i";
        let c = parse_each_as_clause(clause, 0);
        // Top-level comma inside `{ ... }` must be ignored — the outer
        // comma after `}` separates context from index.
        assert_eq!(c.context_range.slice(clause), "{ a, b }");
        assert_eq!(c.index_range.map(|r| r.slice(clause)), Some("i"));
    }
}
