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
use crate::mustache::{
    MustacheSigil, can_start_regex, classify_mustache_sigil, find_mustache_end, skip_ascii_string,
    skip_regex_literal, skip_template_literal,
};
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
    if scanner.peek_byte() != Some(b'{') {
        return None;
    }
    // Whitespace is allowed between `{` and the `:`/`/` sigil (upstream
    // allows whitespace before classifying). A `/` that begins a comment
    // is an expression, not a close tag — the classifier handles that.
    let (sigil, sigil_idx) = classify_mustache_sigil(scanner.source().as_bytes(), start as usize);
    let is_close = match sigil {
        MustacheSigil::BlockClose => true,
        MustacheSigil::Continuation => false,
        _ => return None,
    };
    scanner.set_pos(sigil_idx as u32 + 1); // past `:` or `/`

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
            } else if scanner.starts_with("if")
                // Require a word boundary after `if` so `{:else iffy}`
                // isn't read as `{:else if}` with condition `fy`. The
                // keyword ends only when the next char can't continue an
                // identifier (whitespace, `{`, `(`, etc.).
                && !scanner
                    .peek_byte_at(2)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
            {
                scanner.advance(2);
                scanner.skip_ascii_whitespace();
                let cond_start = scanner.pos();
                let end = find_matching_close_brace(scanner)?;
                if end <= cond_start {
                    errors.push(ParseError::MalformedOpenTag {
                        range: Range::new(cond_start, end.max(cond_start)),
                    });
                    scanner.set_pos(start);
                    return None;
                }
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
            // Optional binding before `}`. Round-8 follow-up #3:
            // destructure patterns (`{:catch { message }}`) contain
            // nested `{...}` braces; scan depth-aware via
            // `find_mustache_end` instead of stopping at the first
            // `}` (which would land inside the destructure pattern).
            let ctx_start = scanner.pos();
            let close_pos = find_mustache_end(scanner.source(), ctx_start);
            let Some(close_pos) = close_pos else {
                errors.push(ParseError::MalformedOpenTag {
                    range: Range::new(start, scanner.pos()),
                });
                scanner.set_pos(start);
                return None;
            };
            let ctx_end = close_pos;
            scanner.set_pos(ctx_end);
            scanner.advance_byte(); // consume the `}`
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
    // Find the top-level `as` that separates the iterable from the
    // binding context. Any whitespace run (spaces, tabs, newlines) may
    // surround it — upstream parses the iterable with `read_expression`,
    // then `allow_whitespace()` + `match('as')`.
    //
    // In TypeScript the iterable itself may contain `as` casts at depth
    // zero (`{#each items as unknown as Item[] as item}`). Upstream
    // acorn-parses the whole header, then peels ONE trailing
    // TSAsExpression back off as the context — i.e. only the LAST `as`
    // whose right side is a valid binding pattern splits the header.
    // Mirror that by trying candidates right-to-left and accepting the
    // first whose suffix parses as `pattern [, index] [(key)]`.
    let mut chosen: Option<(usize, EachAsClause)> = None;
    for p in find_keywords_at_depth_zero(header, "as").into_iter().rev() {
        let clause_start = header_start + p as u32 + 2; // past `as`
        let clause_str = &scanner.source()[clause_start as usize..header_end as usize];
        let clause = parse_each_as_clause(clause_str, clause_start);
        if each_as_clause_is_valid(scanner.source(), &clause) {
            chosen = Some((p, clause));
            break;
        }
    }

    let (expression_range, as_clause) = match chosen {
        Some((p, clause)) => {
            // Trim the whitespace run before `as` off the expression.
            let expr_end = skip_ws_end(header, 0, p as u32) + header_start;
            (Range::new(header_start, expr_end), Some(clause))
        }
        None => parse_each_sequence_form(scanner.source(), header, header_start, header_end),
    };

    scanner.set_pos(header_end + 1);
    Some((expression_range, as_clause))
}

/// `{#each expr, i}` / `{#each expr, i (key)}` — the index-only sequence
/// form (no `as` context). Upstream reads the header as a
/// SequenceExpression, rewinds to the first expression's end, and binds
/// `, i` as the index. Splits at the first depth-0 comma when the
/// remainder is `identifier [(key)]`; otherwise the whole header is the
/// iterable expression.
fn parse_each_sequence_form(
    source: &str,
    header: &str,
    header_start: u32,
    header_end: u32,
) -> (Range, Option<EachAsClause>) {
    let whole = (Range::new(header_start, header_end), None);
    let Some(comma) = find_char_at_depth_zero(header, b',') else {
        return whole;
    };
    let clause_start = header_start + comma as u32 + 1;
    let clause_str = &source[clause_start as usize..header_end as usize];
    let clause = parse_each_as_clause(clause_str, clause_start);
    // The slot before any `(key)` must be a bare identifier — it is the
    // INDEX binding here, not a context pattern (upstream reads it with
    // read_identifier), and it must be present.
    let index_ok = clause.index_range.is_none()
        && clause
            .context_range
            .is_some_and(|r| is_valid_identifier(&source[r.start as usize..r.end as usize]));
    if !index_ok {
        return whole;
    }
    let expr_end = skip_ws_end(header, 0, comma as u32) + header_start;
    (
        Range::new(header_start, expr_end),
        Some(EachAsClause {
            context_range: None,
            index_range: clause.context_range,
            key_range: clause.key_range,
        }),
    )
}

/// Whether a candidate `as`-clause split holds up: the context must
/// parse as a real binding pattern and the index (when present) must be
/// a plain identifier. Rejecting a candidate makes the caller try the
/// next `as` to its left (TS casts) or fall back to no clause at all.
fn each_as_clause_is_valid(source: &str, clause: &EachAsClause) -> bool {
    let Some(ctx) = clause.context_range else {
        return false;
    };
    if !is_valid_binding_pattern(&source[ctx.start as usize..ctx.end as usize]) {
        return false;
    }
    match clause.index_range {
        Some(idx) => is_valid_identifier(&source[idx.start as usize..idx.end as usize]),
        None => true,
    }
}

/// True when `pattern` parses as a complete `let` binding pattern —
/// an identifier or (possibly nested) destructuring with defaults.
/// Validated with oxc in TS mode so both plain and TypeScript headers
/// accept the same shapes.
fn is_valid_binding_pattern(pattern: &str) -> bool {
    if pattern.trim().is_empty() {
        return false;
    }
    let allocator = oxc_allocator::Allocator::default();
    let probe = format!("let {pattern} = 0;");
    let parsed = oxc_parser::Parser::new(
        &allocator,
        &probe,
        oxc_span::SourceType::default().with_typescript(true),
    )
    .parse();
    parsed.diagnostics.is_empty() && !parsed.panicked && parsed.program.body.len() == 1
}

/// Plain-identifier check for index bindings (`i` in `as item, i`).
fn is_valid_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
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
    let context_range = if context_start < context_end {
        Some(Range::new(
            base_offset + context_start,
            base_offset + context_end,
        ))
    } else {
        None
    };

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

    // Look for the `then` or `catch` separator keyword at depth zero. Any
    // whitespace run may surround it (upstream: read_expression +
    // allow_whitespace + eat('then')), and the trailing context binding is
    // optional (`{#await p then}` is the no-binding shorthand).
    //
    // `then` and `catch` are also valid identifiers, so an operand merely
    // NAMED `then` ({#await flag ? then : fallback}) must stay inside the
    // promise expression — upstream acorn-parses the expression first and
    // only then eats the keyword. Each candidate is therefore checked
    // with [`is_await_branch_split`] before it is allowed to split.
    let find_branch_keyword = |kw: &str| {
        find_keywords_at_depth_zero(header, kw)
            .into_iter()
            .find(|&p| is_await_branch_split(header, p, kw.len()))
    };
    let (short, split_at) = if let Some(p) = find_branch_keyword("then") {
        let ctx_start = header_start + (p + 4) as u32;
        let ctx = trimmed_range(scanner.source(), ctx_start, header_end);
        (AwaitShortForm::Then(ctx), Some(p))
    } else if let Some(p) = find_branch_keyword("catch") {
        let ctx_start = header_start + (p + 5) as u32;
        let ctx = trimmed_range(scanner.source(), ctx_start, header_end);
        (AwaitShortForm::Catch(ctx), Some(p))
    } else {
        (AwaitShortForm::None, None)
    };

    let expression_range = match split_at {
        // Trim the whitespace run before the keyword off the expression.
        Some(p) => Range::new(
            header_start,
            header_start + skip_ws_end(header, 0, p as u32),
        ),
        None => Range::new(header_start, header_end),
    };

    scanner.set_pos(header_end + 1);
    Some((expression_range, short))
}

/// Whether the `then`/`catch` keyword at `p` in an await header is the
/// branch separator rather than an identifier inside the promise
/// expression. Approximates upstream's parse-expression-first order with
/// two shape checks: the last significant byte before the keyword must
/// be able to END an expression (identifier/literal tail, closing
/// bracket, quote, backtick, or TS non-null `!`), and what follows the
/// keyword must start a binding pattern — or the header must end there
/// (`{#await p then}` has no binding).
fn is_await_branch_split(header: &str, p: usize, kw_len: usize) -> bool {
    let bytes = header.as_bytes();
    let mut i = p;
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    if i == 0 {
        // Nothing before the keyword: `{#await then}` awaits a variable
        // NAMED `then`, it isn't the branch shorthand.
        return false;
    }
    let prev = bytes[i - 1];
    let prev_can_end_expression = prev.is_ascii_alphanumeric()
        || prev >= 0x80
        || matches!(
            prev,
            b'_' | b'$' | b')' | b']' | b'}' | b'"' | b'\'' | b'`' | b'!'
        );
    if !prev_can_end_expression {
        return false;
    }
    let mut j = p + kw_len;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    match bytes.get(j) {
        None => true,
        Some(&b) => b.is_ascii_alphabetic() || b >= 0x80 || matches!(b, b'_' | b'$' | b'{' | b'['),
    }
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

/// `{#snippet name(params)}` / `{#snippet name<T>(params)}` header.
///
/// Returns `(name, params_range, generics_range)` where `params_range`
/// is the byte range of the parameter list (inside the parens, not
/// including them) and `generics_range` is the byte range of the
/// optional TS type-parameter list (inside the `<>`, not including
/// them).
pub(crate) fn parse_snippet_header(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
) -> Option<(SmolStr, Range, Option<Range>)> {
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

    // Optional TS generic signature between the name and the parameter
    // list: `{#snippet row<T>(x: T)}` (Svelte 5.19+). Upstream matches
    // a balanced `<...>` (match_bracket with `{'<': '>'}`) that skips
    // quoted strings; nested angle pairs count, everything else doesn't.
    let mut generics_range: Option<Range> = None;
    if scanner.peek_byte() == Some(b'<') {
        let open = scanner.pos();
        let Some(close) = find_matching_angle_bracket(scanner.source(), open) else {
            errors.push(ParseError::MalformedOpenTag {
                range: Range::new(open, scanner.source().len() as u32),
            });
            return None;
        };
        generics_range = Some(Range::new(open + 1, close));
        scanner.set_pos(close + 1);
        scanner.skip_ascii_whitespace();
    }

    if scanner.peek_byte() != Some(b'(') {
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(scanner.pos(), scanner.pos()),
        });
        return None;
    }
    scanner.advance_byte();
    let params_start = scanner.pos();

    // Find matching `)`. Strings, char literals, template literals (with
    // their `${...}` interpolations), regex literals, and comments are all
    // skipped so a `)` inside any of them doesn't close the params early —
    // mirroring `find_mustache_end`'s `}`-finding lexical tracking.
    let bytes = scanner.source().as_bytes();
    let mut i = params_start as usize;
    let mut paren_depth: i32 = 1;
    // Brace depth + template stack track `${...}` interpolations exactly as
    // in `find_mustache_end`, so a `)` inside an interpolation is governed by
    // its own balanced braces rather than the outer paren count.
    let mut brace_depth: i32 = 0;
    let mut template_brace_stack: Vec<i32> = Vec::new();
    let mut close_paren: Option<usize> = None;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                paren_depth += 1;
                i += 1;
            }
            b')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    close_paren = Some(i);
                    break;
                }
                i += 1;
            }
            b'{' => {
                brace_depth += 1;
                i += 1;
            }
            b'}' => {
                brace_depth -= 1;
                if let Some(&top) = template_brace_stack.last() {
                    if brace_depth == top {
                        // Returning from `${...}` into the template literal.
                        template_brace_stack.pop();
                        i += 1;
                        i = skip_template_literal(
                            bytes,
                            i,
                            &mut template_brace_stack,
                            &mut brace_depth,
                        )?;
                        continue;
                    }
                }
                i += 1;
            }
            b'"' => i = skip_ascii_string(bytes, i + 1, b'"')?,
            b'\'' => i = skip_ascii_string(bytes, i + 1, b'\'')?,
            b'`' => {
                i = skip_template_literal(
                    bytes,
                    i + 1,
                    &mut template_brace_stack,
                    &mut brace_depth,
                )?
            }
            b'/' => match bytes.get(i + 1).copied() {
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
                _ if can_start_regex(bytes, params_start as usize, i) => {
                    i = skip_regex_literal(bytes, i + 1)?;
                }
                _ => i += 1,
            },
            _ => i += 1,
        }
    }
    let params_end = close_paren
        .map(|p| p as u32)
        .unwrap_or(scanner.source().len() as u32);
    scanner.set_pos(params_end);
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

    Some((name, Range::new(params_start, params_end), generics_range))
}

/// Find the `>` matching the `<` at `open`, counting nested angle pairs
/// and skipping quoted/template strings — the same shape as upstream's
/// `match_bracket` with the `{'<': '>'}` bracket set. Returns the byte
/// offset of the matching `>`.
fn find_matching_angle_bracket(src: &str, open: u32) -> Option<u32> {
    let bytes = src.as_bytes();
    debug_assert_eq!(bytes.get(open as usize), Some(&b'<'));
    let mut depth: i32 = 0;
    let mut i = open as usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                depth += 1;
                i += 1;
            }
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i as u32);
                }
                i += 1;
            }
            b'"' => i = skip_ascii_string(bytes, i + 1, b'"')?,
            b'\'' => i = skip_ascii_string(bytes, i + 1, b'\'')?,
            b'`' => {
                // Template-literal type: scan to the closing backtick
                // (may span lines, unlike the quote forms).
                i += 1;
                while i < bytes.len() && bytes[i] != b'`' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                if i >= bytes.len() {
                    return None;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

// ===== Lexical helpers ===================================================

/// Find every standalone keyword occurrence (`as` / `then` / `catch`)
/// at depth 0 (outside strings / template literals / nested parens /
/// brackets / braces / comments). The keyword matches only when preceded
/// by at least one whitespace byte and followed by whitespace or the end
/// of `src` — mirroring upstream's `read_expression`,
/// `allow_whitespace()`, `match(keyword)` sequence, which accepts any
/// whitespace run (spaces, tabs, CR/LF) around the keyword, not just
/// single spaces. Returns the byte offsets of the keyword itself, in
/// source order.
fn find_keywords_at_depth_zero(src: &str, keyword: &str) -> Vec<usize> {
    let bytes = src.as_bytes();
    let kw = keyword.as_bytes();
    let mut i = 0;
    let mut depth: i32 = 0;
    let mut found = Vec::new();

    while i < bytes.len() {
        if depth == 0
            && i > 0
            && bytes[i - 1].is_ascii_whitespace()
            && bytes[i..].starts_with(kw)
            && bytes
                .get(i + kw.len())
                .is_none_or(|b| b.is_ascii_whitespace())
        {
            found.push(i);
            i += kw.len();
            continue;
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
            b'/' => match bytes.get(i + 1).copied() {
                Some(b'/') => {
                    // Line comment to end of line. `//` is unambiguously a
                    // comment in expression context (an empty `//` regex is
                    // invalid), so no regex/division disambiguation is needed.
                    i += 2;
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                Some(b'*') => {
                    // Block comment to `*/`. Leave `i` on the closing `/` so
                    // the trailing `i += 1` steps past it.
                    i += 2;
                    while i + 1 < bytes.len() {
                        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => {}
            },
            _ => {}
        }
        i += 1;
    }
    found
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
            b'/' => match bytes.get(i + 1).copied() {
                Some(b'/') => {
                    // Line comment to end of line. `//` is unambiguously a
                    // comment in expression context (an empty `//` regex is
                    // invalid), so no regex/division disambiguation is needed.
                    i += 2;
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                Some(b'*') => {
                    // Block comment to `*/`. Leave `i` on the closing `/` so
                    // the trailing `i += 1` steps past it.
                    i += 2;
                    while i + 1 < bytes.len() {
                        if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => {}
            },
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
    generics_range: Option<Range>,
    body: Fragment,
    block_start: u32,
    block_end: u32,
) -> SnippetBlock {
    SnippetBlock {
        name,
        parameters_range,
        generics_range,
        body,
        range: Range::new(block_start, block_end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn else_if_requires_word_boundary() {
        // `{:else iffy}` must NOT be read as `{:else if}` with condition
        // `fy` — `iffy` is not the `if` keyword.
        let src = "{:else iffy}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let term = peek_and_consume_terminator(&mut scanner, &mut errors);
        assert!(
            !matches!(term, Some(BlockTerminator::ElseIf { .. })),
            "got {term:?}"
        );
        assert!(!errors.is_empty(), "expected a malformed-open-tag error");
    }

    #[test]
    fn terminator_allows_whitespace_after_open_brace() {
        // Upstream allows whitespace between `{` and the `:`/`/` sigil.
        for (src, expect_close) in [
            ("{ /if}", true),
            ("{\t/each}", true),
            ("{\n  /await}", true),
            ("{ :else}", false),
        ] {
            let mut scanner = Scanner::new(src);
            let mut errors = Vec::new();
            let term = peek_and_consume_terminator(&mut scanner, &mut errors);
            match (expect_close, term) {
                (true, Some(BlockTerminator::Close { .. }))
                | (false, Some(BlockTerminator::Else)) => {}
                (_, other) => panic!("wrong terminator for {src:?}: {other:?}"),
            }
            assert!(
                errors.is_empty(),
                "unexpected errors for {src:?}: {errors:?}"
            );
        }
    }

    #[test]
    fn brace_comment_is_not_a_terminator() {
        // `{//x}` and `{/* x */}` start with `{/` but the `/` opens a
        // comment — they are expressions, not close tags.
        for src in ["{//x}", "{/* x */ y}", "{ /* x */ y}"] {
            let mut scanner = Scanner::new(src);
            let mut errors = Vec::new();
            let term = peek_and_consume_terminator(&mut scanner, &mut errors);
            assert!(
                term.is_none(),
                "expected no terminator for {src:?}, got {term:?}"
            );
            assert_eq!(
                scanner.pos(),
                0,
                "scanner must be left untouched for {src:?}"
            );
            assert!(
                errors.is_empty(),
                "unexpected errors for {src:?}: {errors:?}"
            );
        }
    }

    #[test]
    fn else_if_real_condition_parses() {
        let src = "{:else if ready}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let term = peek_and_consume_terminator(&mut scanner, &mut errors);
        let Some(BlockTerminator::ElseIf { condition_range }) = term else {
            panic!("expected ElseIf, got {term:?}");
        };
        assert_eq!(condition_range.slice(src), "ready");
    }

    #[test]
    fn else_if_empty_condition_is_malformed() {
        // `{:else if}` has no condition — must not produce an ElseIf.
        let src = "{:else if}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let term = peek_and_consume_terminator(&mut scanner, &mut errors);
        assert!(
            !matches!(term, Some(BlockTerminator::ElseIf { .. })),
            "got {term:?}"
        );
        assert!(!errors.is_empty(), "expected a malformed-open-tag error");
    }

    #[test]
    fn snippet_header_accepts_generics() {
        // `{#snippet row<T>(x: T)}` — TS generic signature between the
        // name and the parameter list (Svelte 5.19+).
        let src = "row<T>(x: T)}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (name, params, generics) =
            parse_snippet_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(name, "row");
        assert_eq!(params.slice(src), "x: T");
        assert_eq!(generics.map(|g| g.slice(src)), Some("T"));
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn snippet_generics_nested_angles_and_strings() {
        let src = "row<A, B extends Map<A, 'x>y'>>(a: A, b: B)}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (name, params, generics) =
            parse_snippet_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(name, "row");
        assert_eq!(
            generics.map(|g| g.slice(src)),
            Some("A, B extends Map<A, 'x>y'>")
        );
        assert_eq!(params.slice(src), "a: A, b: B");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn snippet_header_without_generics_unchanged() {
        let src = "item(x)}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (name, params, generics) =
            parse_snippet_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(name, "item");
        assert_eq!(params.slice(src), "x");
        assert!(generics.is_none());
    }

    #[test]
    fn snippet_params_are_string_aware() {
        // `)` inside a default-value string must not close the params
        // early: `{#snippet s(a = ")")}`.
        let src = r#"s(a = ")")}"#;
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let result = parse_snippet_header(&mut scanner, &mut errors);
        let Some((name, params, _)) = result else {
            panic!("expected a snippet header, errors: {errors:?}");
        };
        assert_eq!(name, "s");
        assert_eq!(params.slice(src), r#"a = ")""#);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn snippet_params_are_regex_aware() {
        // `)` inside a regex literal must not close the params early.
        let src = "s(a = /)/)}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let result = parse_snippet_header(&mut scanner, &mut errors);
        let Some((name, params, _)) = result else {
            panic!("expected a snippet header, errors: {errors:?}");
        };
        assert_eq!(name, "s");
        assert_eq!(params.slice(src), "a = /)/");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn snippet_params_are_block_comment_aware() {
        // `)` inside a block comment must not close the params early.
        let src = "s(a /* ) */)}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let result = parse_snippet_header(&mut scanner, &mut errors);
        let Some((name, params, _)) = result else {
            panic!("expected a snippet header, errors: {errors:?}");
        };
        assert_eq!(name, "s");
        assert_eq!(params.slice(src), "a /* ) */");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn await_header_accepts_newline_around_then_and_catch() {
        // `{#await p\nthen v}` / `{#await p\tcatch e}` are valid Svelte —
        // upstream reads the promise expression, then allow_whitespace +
        // eat('then'/'catch'), so any whitespace run may separate them.
        for (src, expect_expr, expect_then) in [
            ("p\nthen v}", "p", true),
            ("p\tthen\tv}", "p", true),
            ("p\r\nthen v}", "p", true),
            ("p \n then \n v}", "p", true),
            ("p\ncatch v}", "p", false),
            ("p\tcatch\tv}", "p", false),
        ] {
            let mut scanner = Scanner::new(src);
            let mut errors = Vec::new();
            let Some((expr, short)) = parse_await_header(&mut scanner, &mut errors) else {
                panic!("expected await header for {src:?}, errors: {errors:?}");
            };
            assert_eq!(expr.slice(src), expect_expr, "expression for {src:?}");
            match (expect_then, short) {
                (true, AwaitShortForm::Then(ctx)) | (false, AwaitShortForm::Catch(ctx)) => {
                    let ctx = ctx.unwrap_or_else(|| panic!("binding dropped for {src:?}"));
                    assert_eq!(ctx.slice(src), "v", "binding for {src:?}");
                }
                (_, other) => panic!("wrong short form for {src:?}: {other:?}"),
            }
            assert!(
                errors.is_empty(),
                "unexpected errors for {src:?}: {errors:?}"
            );
        }
    }

    #[test]
    fn await_header_operand_named_then_is_not_a_split() {
        // `{#await flag ? then : fallback}` awaits a conditional whose
        // branches are variables NAMED `then`/`fallback` — upstream
        // parses the whole expression first, so no branch split happens.
        let src = "flag ? then : fallback}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, short) = parse_await_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "flag ? then : fallback");
        assert!(matches!(short, AwaitShortForm::None), "got {short:?}");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn await_header_bare_then_is_an_expression() {
        // `{#await then}` awaits a variable named `then` (upstream reads
        // it with read_expression before trying to eat the keyword).
        let src = "then}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, short) = parse_await_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "then");
        assert!(matches!(short, AwaitShortForm::None), "got {short:?}");
    }

    #[test]
    fn await_header_second_then_candidate_splits() {
        // The first ` then ` sits after `?` (can't end an expression);
        // the second one, after an identifier, is the real separator.
        let src = "a ? then : b then v}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, short) = parse_await_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "a ? then : b");
        let AwaitShortForm::Then(Some(ctx)) = short else {
            panic!("expected then shorthand, got {short:?}");
        };
        assert_eq!(ctx.slice(src), "v");
    }

    #[test]
    fn await_header_nonnull_assertion_before_then_splits() {
        // TS postfix `!` can end the promise expression: `{#await p! then v}`.
        let src = "p! then v}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, short) = parse_await_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "p!");
        let AwaitShortForm::Then(Some(ctx)) = short else {
            panic!("expected then shorthand, got {short:?}");
        };
        assert_eq!(ctx.slice(src), "v");
    }

    #[test]
    fn await_header_multiline_promise_expression() {
        let src = "fetchData(\n  url,\n  opts\n)\nthen result}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, short) = parse_await_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "fetchData(\n  url,\n  opts\n)");
        let AwaitShortForm::Then(Some(ctx)) = short else {
            panic!("expected then shorthand, got {short:?}");
        };
        assert_eq!(ctx.slice(src), "result");
    }

    #[test]
    fn await_header_newline_then_without_binding() {
        // `{#await p\nthen}` — keyword at end of header, no binding.
        let src = "p\nthen}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, short) = parse_await_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "p");
        assert!(matches!(short, AwaitShortForm::Then(None)), "got {short:?}");
    }

    /// First-match convenience over [`find_keywords_at_depth_zero`],
    /// mirroring how header parsing consumes the candidate list.
    fn find_keyword_at_depth_zero(src: &str, keyword: &str) -> Option<usize> {
        find_keywords_at_depth_zero(src, keyword).into_iter().next()
    }

    #[test]
    fn find_keyword_at_depth_zero_whitespace_variants() {
        // Any whitespace run around the keyword matches — upstream does
        // read_expression + allow_whitespace + match(keyword).
        assert_eq!(find_keyword_at_depth_zero("foo as bar", "as"), Some(4));
        assert_eq!(find_keyword_at_depth_zero("foo\nas bar", "as"), Some(4));
        assert_eq!(find_keyword_at_depth_zero("foo\tas\tbar", "as"), Some(4));
        assert_eq!(find_keyword_at_depth_zero("foo\r\nas bar", "as"), Some(5));
        assert_eq!(find_keyword_at_depth_zero("foo  as\n bar", "as"), Some(5));
        // Keyword at end of src (no binding after it).
        assert_eq!(find_keyword_at_depth_zero("p then", "then"), Some(2));
        // Not a standalone keyword: part of an identifier.
        assert_eq!(find_keyword_at_depth_zero("foo aster", "as"), None);
        assert_eq!(find_keyword_at_depth_zero("foo.as bar", "as"), None);
        // Inside parens: depth != 0, skip to the top-level one.
        assert_eq!(
            find_keyword_at_depth_zero("foo(a as b)\nas c", "as"),
            Some(12)
        );
        // Inside a string: skipped.
        assert_eq!(find_keyword_at_depth_zero("\"x as y\" as z", "as"), Some(9));
    }

    #[test]
    fn each_header_accepts_newline_around_as() {
        // `{#each items\nas item}` is valid Svelte — the as-clause must
        // not be silently dropped when the whitespace isn't a single
        // space.
        for src in [
            "items\nas item}",
            "items\tas\titem}",
            "items\r\nas item}",
            "items \n as item}",
        ] {
            let mut scanner = Scanner::new(src);
            let mut errors = Vec::new();
            let Some((expr, clause)) = parse_each_header(&mut scanner, &mut errors) else {
                panic!("expected each header for {src:?}, errors: {errors:?}");
            };
            assert_eq!(expr.slice(src), "items", "expression for {src:?}");
            let clause = clause.unwrap_or_else(|| panic!("as-clause dropped for {src:?}"));
            assert_eq!(
                clause.context_range.unwrap().slice(src),
                "item",
                "context for {src:?}"
            );
            assert!(
                errors.is_empty(),
                "unexpected errors for {src:?}: {errors:?}"
            );
        }
    }

    #[test]
    fn each_header_multiline_expression_with_as() {
        let src = "getItems(\n  a,\n  b\n)\nas item, i (item.id)}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "getItems(\n  a,\n  b\n)");
        let clause = clause.expect("as-clause present");
        assert_eq!(clause.context_range.unwrap().slice(src), "item");
        assert_eq!(clause.index_range.map(|r| r.slice(src)), Some("i"));
        assert_eq!(clause.key_range.map(|r| r.slice(src)), Some("item.id"));
    }

    #[test]
    fn each_header_ts_casts_stay_in_expression() {
        // `{#each items as unknown as Item[] as item}` — the first two
        // `as` are TypeScript casts belonging to the iterable; only the
        // LAST `as` (whose right side is a valid binding pattern)
        // separates the context. Upstream acorn-parses the whole header
        // and peels one trailing TSAsExpression back off.
        let src = "items as unknown as Item[] as item}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "items as unknown as Item[]");
        let clause = clause.expect("as-clause present");
        assert_eq!(clause.context_range.unwrap().slice(src), "item");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn each_header_single_ts_cast_with_context_and_index() {
        let src = "items as Item[] as item, i (item.id)}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "items as Item[]");
        let clause = clause.expect("as-clause present");
        assert_eq!(clause.context_range.unwrap().slice(src), "item");
        assert_eq!(clause.index_range.map(|r| r.slice(src)), Some("i"));
        assert_eq!(clause.key_range.map(|r| r.slice(src)), Some("item.id"));
    }

    #[test]
    fn each_header_sequence_form_binds_index_without_context() {
        // `{#each expr, i}` — index-only form (no `as`). Upstream reads
        // the header as a SequenceExpression, keeps the first expression
        // as the iterable, and binds `i` as the index.
        let src = "Array.from({ length: 10 }), i}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "Array.from({ length: 10 })");
        let clause = clause.expect("sequence-form clause present");
        assert!(
            clause.context_range.is_none(),
            "no context in sequence form"
        );
        assert_eq!(clause.index_range.map(|r| r.slice(src)), Some("i"));
        assert!(clause.key_range.is_none());
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn each_header_sequence_form_with_key() {
        let src = "foo(a, b), i (i)}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "foo(a, b)");
        let clause = clause.expect("sequence-form clause present");
        assert!(clause.context_range.is_none());
        assert_eq!(clause.index_range.map(|r| r.slice(src)), Some("i"));
        assert_eq!(clause.key_range.map(|r| r.slice(src)), Some("i"));
    }

    #[test]
    fn each_header_comma_in_call_is_not_sequence_form() {
        // The comma inside `f(a, b)` is at depth > 0 — no index binding.
        let src = "f(a, b)}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "f(a, b)");
        assert!(clause.is_none(), "no clause expected, got {clause:?}");
    }

    #[test]
    fn each_header_destructure_with_default_still_splits() {
        // `as { y = z }` is not a valid expression, only a pattern —
        // the split must still be found (upstream backtracks to the
        // `as` when acorn fails on it).
        let src = "list as { y = z }}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "list");
        let clause = clause.expect("as-clause present");
        assert_eq!(clause.context_range.unwrap().slice(src), "{ y = z }");
    }

    #[test]
    fn find_keywords_at_depth_zero_returns_all() {
        assert_eq!(find_keywords_at_depth_zero("a as b as c", "as"), vec![2, 7]);
        assert_eq!(find_keywords_at_depth_zero("(a as b) as c", "as"), vec![9]);
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
        assert_eq!(c.context_range, Some(Range::new(10, 14)));
        assert!(c.index_range.is_none());
        assert!(c.key_range.is_none());
    }

    #[test]
    fn parse_each_as_clause_with_index() {
        let clause = "item, i";
        let c = parse_each_as_clause(clause, 0);
        assert_eq!(c.context_range.unwrap().slice(clause), "item");
        assert_eq!(c.index_range.map(|r| r.slice(clause)), Some("i"));
    }

    #[test]
    fn parse_each_as_clause_with_key() {
        let clause = "item (item.id)";
        let c = parse_each_as_clause(clause, 0);
        assert_eq!(c.context_range.unwrap().slice(clause), "item");
        assert_eq!(c.key_range.map(|r| r.slice(clause)), Some("item.id"));
    }

    #[test]
    fn parse_each_as_clause_full() {
        let clause = "item, i (item.id)";
        let c = parse_each_as_clause(clause, 0);
        assert_eq!(c.context_range.unwrap().slice(clause), "item");
        assert_eq!(c.index_range.map(|r| r.slice(clause)), Some("i"));
        assert_eq!(c.key_range.map(|r| r.slice(clause)), Some("item.id"));
    }

    #[test]
    fn parse_each_as_clause_destructuring() {
        let clause = "{ a, b }, i";
        let c = parse_each_as_clause(clause, 0);
        // Top-level comma inside `{ ... }` must be ignored — the outer
        // comma after `}` separates context from index.
        assert_eq!(c.context_range.unwrap().slice(clause), "{ a, b }");
        assert_eq!(c.index_range.map(|r| r.slice(clause)), Some("i"));
    }
}
