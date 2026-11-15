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

use svn_core::Range;

use crate::error::ParseError;
use crate::mustache::{MustacheSigil, classify_mustache_sigil, find_mustache_end};
use crate::scanner::{Scanner, is_svelte_whitespace_before};
use crate::tag_read::{TagReadError, TagReader};

/// What block terminator was found in a child fragment scan.
#[derive(Debug)]
pub(crate) enum BlockTerminator {
    /// `{/if}`, `{/each}`, `{/await}`, `{/key}`, `{/snippet}`. `tag` is the
    /// block name without `/`. Scanner positioned just past the terminator.
    Close { tag: String },
    /// `{:else}` — no trailing condition.
    Else,
    /// `{:else if condition}` — trailing expression range captured, and
    /// the error the compiler throws reading the tag.
    ElseIf {
        condition_range: Range,
        error: Option<TagReadError>,
    },
    /// `{:then [binding]}` — optional binding context range, and the
    /// error the compiler throws reading the tag.
    Then {
        context_range: Option<Range>,
        error: Option<TagReadError>,
    },
    /// `{:catch [binding]}` — the same for `catch`.
    Catch {
        context_range: Option<Range>,
        error: Option<TagReadError>,
    },
}

/// Try to recognize a branch tag (`{:...}`) or close tag (`{/...}`) at the
/// scanner's position. Returns the terminator and advances past it on
/// success; leaves the scanner unchanged on non-match.
pub(crate) fn peek_and_consume_terminator(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
    reader: &TagReader<'_>,
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
        scanner.skip_svelte_whitespace();
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
            scanner.skip_svelte_whitespace();
            if scanner.peek_byte() == Some(b'}') {
                scanner.advance_byte();
                Some(BlockTerminator::Else)
            } else if scanner.starts_with("if") {
                // The compiler takes `if` by prefix and then requires
                // whitespace: `{:else iffy}` is an `{:else if}` missing
                // it.
                scanner.advance(2);
                let limit = scanner.source().len() as u32;
                let header = read_expression_header(scanner, reader, limit);
                Some(BlockTerminator::ElseIf {
                    condition_range: header.range,
                    error: header.error,
                })
            } else {
                errors.push(ParseError::MalformedOpenTag {
                    range: Range::new(start, scanner.pos()),
                });
                scanner.set_pos(start);
                None
            }
        }
        "then" | "catch" => {
            // The tag ends where the compiler's reading of its value ends;
            // no read goes past the `}` a brace count finds.
            let keyword_end = word_end;
            let counted_close = find_mustache_end(scanner.source(), keyword_end);
            let limit = counted_close.unwrap_or(scanner.source().len() as u32);
            let (context_range, error) = match reader.read_branch(keyword_end, limit) {
                Ok(branch) => {
                    scanner.set_pos(branch.end);
                    (branch.value, None)
                }
                Err(error) => {
                    // The compiler throws, and the component is rejected.
                    // The tag ends at the counted `}`, or where the error
                    // is.
                    scanner.set_pos(counted_close.map_or(error.at.max(keyword_end), |c| c + 1));
                    (None, Some(error))
                }
            };
            if word == "then" {
                Some(BlockTerminator::Then {
                    context_range,
                    error,
                })
            } else {
                Some(BlockTerminator::Catch {
                    context_range,
                    error,
                })
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

fn skip_ws_start(src: &str, start: u32, end: u32) -> u32 {
    (crate::scanner::skip_svelte_whitespace_at(src.as_bytes(), start as usize) as u32).min(end)
}

fn skip_ws_end(src: &str, start: u32, mut end: u32) -> u32 {
    let bytes = src.as_bytes();
    while end > start && is_svelte_whitespace_before(bytes, end as usize) {
        // Step back over the whole whitespace char (may be multi-byte).
        let mut e = end - 1;
        while e > start && bytes[e as usize] & 0xC0 == 0x80 {
            e -= 1;
        }
        end = e;
    }
    end
}

// ===== Header parsing ====================================================

/// The header of a tag that holds one expression after its keyword.
pub(crate) struct ExpressionHeader {
    /// The expression acorn read; on an error, the text up to the `}`
    /// that closes the tag.
    pub range: Range,
    /// The error the compiler throws reading the tag.
    pub error: Option<TagReadError>,
}

/// Reads the rest of an `{#if`, `{:else if` or `{#key` tag the way the
/// compiler does: whitespace, the expression acorn reads, and the `}` it
/// then expects. The scanner must sit right after the keyword; on return
/// it is just past the tag. `fragment_end` bounds the reading when no
/// `}` closes the tag.
pub(crate) fn read_expression_header(
    scanner: &mut Scanner<'_>,
    reader: &TagReader<'_>,
    fragment_end: u32,
) -> ExpressionHeader {
    let keyword_end = scanner.pos();
    let source = scanner.source();
    // acorn stops at an unpaired `}`, so no read goes past the one a
    // brace count finds.
    let counted_close = find_mustache_end(source, keyword_end);
    let limit = counted_close.unwrap_or(fragment_end);
    let start = reader.skip_whitespace(keyword_end);
    match reader.read_keyword_expression(keyword_end, limit) {
        Ok(read) => {
            scanner.set_pos(read.end);
            ExpressionHeader {
                range: Range::new(start, read.expression_end),
                error: None,
            }
        }
        Err(error) => {
            // The compiler throws, and the component is rejected. The
            // block stays, carrying the error: its tag ends at the
            // counted `}`, or where the error is.
            let end = counted_close.unwrap_or(error.at.max(keyword_end));
            scanner.set_pos(if counted_close.is_some() {
                end + 1
            } else {
                end
            });
            ExpressionHeader {
                range: Range::new(start.min(end), end),
                error: Some(error),
            }
        }
    }
}

// ===== Lexical helpers ===================================================

pub(crate) fn trimmed_range(src: &str, start: u32, end: u32) -> Option<Range> {
    let s = skip_ws_start(src, start, end);
    let e = skip_ws_end(src, s, end);
    if s < e { Some(Range::new(s, e)) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tag_read::TagReadErrorKind;

    #[test]
    fn else_if_keyword_is_read_by_prefix() {
        // The compiler eats `if` wherever the text starts with it and
        // then requires whitespace, so `{:else iffy}` is an `{:else if}`
        // that fails there.
        let src = "{:else iffy}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let term =
            peek_and_consume_terminator(&mut scanner, &mut errors, &TagReader::new(src, false));
        let Some(BlockTerminator::ElseIf {
            error: Some(error), ..
        }) = term
        else {
            panic!("got {term:?}");
        };
        assert_eq!(error.at, 9);
        assert_eq!(error.kind, TagReadErrorKind::ExpectedWhitespace);
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
            let term =
                peek_and_consume_terminator(&mut scanner, &mut errors, &TagReader::new(src, false));
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
            let term =
                peek_and_consume_terminator(&mut scanner, &mut errors, &TagReader::new(src, false));
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
        let term =
            peek_and_consume_terminator(&mut scanner, &mut errors, &TagReader::new(src, false));
        let Some(BlockTerminator::ElseIf {
            condition_range, ..
        }) = term
        else {
            panic!("expected ElseIf, got {term:?}");
        };
        assert_eq!(condition_range.slice(src), "ready");
    }

    #[test]
    fn else_if_without_condition_carries_the_compilers_error() {
        // The compiler requires whitespace after `if` before it reads
        // the condition.
        let src = "{:else if}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let term =
            peek_and_consume_terminator(&mut scanner, &mut errors, &TagReader::new(src, false));
        let Some(BlockTerminator::ElseIf {
            error: Some(error), ..
        }) = term
        else {
            panic!("got {term:?}");
        };
        assert_eq!(error.at, 9);
        assert_eq!(error.kind, TagReadErrorKind::ExpectedWhitespace);
        assert_eq!(scanner.pos(), src.len() as u32);
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
    fn snippet_params_end_at_the_first_balancing_parenthesis() {
        // The compiler counts parentheses one character at a time, in
        // strings, regular expressions and comments too, so each of these
        // ends early and acorn then throws.
        for src in [r#"s(a = ")")}"#, "s(a = /)/)}", "s(a /* ) */)}"] {
            let mut scanner = Scanner::new(src);
            let mut errors = Vec::new();
            assert!(
                parse_snippet_header(&mut scanner, &mut errors).is_none(),
                "{src}"
            );
        }
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
    fn await_header_comment_before_then_splits() {
        // The promise expression is acorn's node: the comments after it
        // are not part of it.
        for (src, expr) in [("p /*c*/ then v}", "p"), ("p/* a */ /* b */ catch e}", "p")] {
            let mut scanner = Scanner::new(src);
            let mut errors = Vec::new();
            let (range, short) =
                parse_await_header(&mut scanner, &mut errors).expect("header parses");
            assert_eq!(range.slice(src), expr);
            assert!(
                matches!(
                    short,
                    AwaitShortForm::Then(Some(_)) | AwaitShortForm::Catch(Some(_))
                ),
                "got {short:?}"
            );
        }
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
    fn each_header_nbsp_around_as() {
        // Upstream is_whitespace accepts NBSP wherever whitespace
        // separates tokens, including around the `as` keyword.
        let src = "xs\u{a0}as\u{a0}x}";
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let (expr, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
        assert_eq!(expr.slice(src), "xs");
        let clause = clause.expect("as-clause present");
        assert_eq!(clause.context_range.unwrap().slice(src), "x");
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
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
    fn each_header_annotation_keeps_its_commas_and_parentheses() {
        // The compiler reads the annotation as acorn-typescript reads
        // `_ as <type>`: the type's own commas and parentheses are part
        // of it, and the index and key come after it. The context is the
        // pattern alone.
        let cases = [
            (
                "entries as entry: Map<string, number>}",
                "entry",
                None,
                None,
            ),
            (
                "handlers as handler: (a: string) => void, i}",
                "handler",
                Some("i"),
                None,
            ),
            (
                "rows as { a }: { a: string, b: number }, i (a)}",
                "{ a }",
                Some("i"),
                Some("a"),
            ),
        ];
        for (src, context, index, key) in cases {
            let mut scanner = Scanner::new(src);
            let mut errors = Vec::new();
            let (_, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
            let clause = clause.expect("as-clause present");
            assert_eq!(
                clause.context_range.map(|r| r.slice(src)),
                Some(context),
                "{src}"
            );
            assert_eq!(clause.index_range.map(|r| r.slice(src)), index, "{src}");
            assert_eq!(clause.key_range.map(|r| r.slice(src)), key, "{src}");
        }
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
    fn each_header_clause_parts() {
        let cases = [
            ("items as item}", "item", None, None),
            ("items as item, i}", "item", Some("i"), None),
            ("items as item (item.id)}", "item", None, Some("item.id")),
            (
                "items as item, i (item.id)}",
                "item",
                Some("i"),
                Some("item.id"),
            ),
            // The comma inside the pattern is the pattern's.
            ("items as { a, b }, i}", "{ a, b }", Some("i"), None),
        ];
        for (src, context, index, key) in cases {
            let mut scanner = Scanner::new(src);
            let mut errors = Vec::new();
            let (_, clause) = parse_each_header(&mut scanner, &mut errors).expect("header parses");
            let clause = clause.expect("as-clause present");
            assert_eq!(
                clause.context_range.map(|r| r.slice(src)),
                Some(context),
                "{src}"
            );
            assert_eq!(clause.index_range.map(|r| r.slice(src)), index, "{src}");
            assert_eq!(clause.key_range.map(|r| r.slice(src)), key, "{src}");
        }
    }

    /// The `{#each}` header `header` (up to and including its `}`) as the
    /// template parser reads it in a TypeScript component, with its ranges
    /// in `header`. `None` when the compiler throws reading it.
    fn parse_each_header(
        scanner: &mut Scanner<'_>,
        errors: &mut Vec<ParseError>,
    ) -> Option<(Range, Option<crate::ast::EachAsClause>)> {
        let header = scanner.source();
        let script = "<script lang=\"ts\"></script>";
        let src = format!("{script}{{#each {header}{{/each}}");
        let template = Range::new(script.len() as u32, src.len() as u32);
        let (fragment, parse_errors) = crate::template::parse_template(&src, template);
        errors.extend(parse_errors);
        let Some(crate::ast::Node::EachBlock(block)) = fragment.nodes.into_iter().next() else {
            return None;
        };
        if block.open_error.is_some() {
            return None;
        }
        let offset = script.len() as u32 + "{#each ".len() as u32;
        let shift = |r: Range| Range::new(r.start - offset, r.end - offset);
        let clause = block.as_clause.map(|c| crate::ast::EachAsClause {
            context_range: c.context_range.map(shift),
            index_range: c.index_range.map(shift),
            key_range: c.key_range.map(shift),
        });
        Some((shift(block.expression_range), clause))
    }

    /// A block tag `{#<keyword> <header>{/<keyword>}` parsed in a
    /// TypeScript component; `offset` is where `header` starts.
    fn parse_block(keyword: &str, header: &str) -> (crate::ast::Fragment, u32, String) {
        let script = "<script lang=\"ts\"></script>";
        let src = format!("{script}{{#{keyword} {header}{{/{keyword}}}");
        let template = Range::new(script.len() as u32, src.len() as u32);
        let (fragment, _) = crate::template::parse_template(&src, template);
        let offset = (script.len() + keyword.len() + 3) as u32;
        (fragment, offset, src)
    }

    #[derive(Debug)]
    enum AwaitShortForm {
        None,
        Then(Option<Range>),
        Catch(Option<Range>),
    }

    /// The `{#await}` header `header` (up to and including its `}`) as
    /// the template parser reads it, with ranges in `header`.
    fn parse_await_header(
        scanner: &mut Scanner<'_>,
        _errors: &mut Vec<ParseError>,
    ) -> Option<(Range, AwaitShortForm)> {
        let (fragment, offset, _) = parse_block("await", scanner.source());
        let Some(crate::ast::Node::AwaitBlock(block)) = fragment.nodes.into_iter().next() else {
            return None;
        };
        if block.open_error.is_some() {
            return None;
        }
        let shift = |r: Range| Range::new(r.start - offset, r.end - offset);
        let short = match (&block.pending, &block.then_branch, &block.catch_branch) {
            (Some(_), _, _) => AwaitShortForm::None,
            (None, Some(then), _) => AwaitShortForm::Then(then.context_range.map(shift)),
            (None, None, Some(catch)) => AwaitShortForm::Catch(catch.context_range.map(shift)),
            _ => return None,
        };
        Some((shift(block.expression_range), short))
    }

    /// The `{#snippet}` header `header` (up to and including its `}`) as
    /// the template parser reads it, with ranges in `header`.
    fn parse_snippet_header(
        scanner: &mut Scanner<'_>,
        _errors: &mut Vec<ParseError>,
    ) -> Option<(String, Range, Option<Range>)> {
        let (fragment, offset, _) = parse_block("snippet", scanner.source());
        let Some(crate::ast::Node::SnippetBlock(block)) = fragment.nodes.into_iter().next() else {
            return None;
        };
        if block.open_error.is_some() {
            return None;
        }
        let shift = |r: Range| Range::new(r.start - offset, r.end - offset);
        Some((
            block.name.to_string(),
            shift(block.parameters_range),
            block.generics_range.map(shift),
        ))
    }
}
