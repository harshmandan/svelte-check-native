//! acorn's reads of template JavaScript, replayed with oxc.
//!
//! The Svelte compiler hands parts of a tag to acorn (acorn-typescript in
//! a TypeScript component) at a position, and acorn reads the longest
//! expression it can from there: `parseExpressionAt`. Where that read
//! stops decides where the compiler's own character-level reading
//! carries on, so both our parser (to split a tag the way the compiler
//! does) and the lint (to report the compiler's errors) replay it.
//!
//! oxc's `parse_expression` reads the same longest expression and then
//! reports the token after it as unexpected. So an error after which the
//! text read so far is a complete expression is where acorn stops; any
//! other error is one acorn throws.

use oxc_allocator::Allocator;
use oxc_ast::ast::Expression;
use oxc_span::{GetSpan, SourceType};

/// What acorn's `parseExpressionAt` does with the text at the start of a
/// string, in that string's positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    /// It read an expression ending at `end`, having looked ahead to the
    /// token at `lookahead`: the comments before that token are among
    /// those it collected.
    Expression { end: u32, lookahead: u32 },
    /// It threw `message` at `at`, having collected the comments before
    /// it.
    Error { at: u32, message: &'static str },
}

/// oxc's message for a TypeScript cast it read without TypeScript.
const CAST_WITHOUT_TYPESCRIPT: &str =
    "Type assertion expressions can only be used in TypeScript files.";

/// acorn's message for the error oxc reports as `oxc`.
fn acorn_message(oxc: &str) -> &'static str {
    match oxc {
        "Cannot assign to this expression" => "Assigning to rvalue",
        _ => "Unexpected token",
    }
}

fn source_type(ts: bool) -> SourceType {
    SourceType::default().with_module(true).with_typescript(ts)
}

fn unparenthesized<'a, 'b>(mut expression: &'a Expression<'b>) -> &'a Expression<'b> {
    while let Expression::ParenthesizedExpression(p) = expression {
        expression = &p.expression;
    }
    expression
}

/// Where `text` ends when it reads as one complete expression.
pub fn complete_expression_end(text: &str, ts: bool) -> Option<u32> {
    let alloc = Allocator::default();
    oxc_parser::Parser::new(&alloc, text, source_type(ts))
        .parse_expression()
        .ok()
        .map(|expression| expression.span().end)
}

/// acorn's read of the expression at the start of `text`.
pub fn read_expression(text: &str, ts: bool) -> Read {
    if !ts
        && let Some(at) = cast_keyword(text, 0, ts)
        && let Some(end) = complete_expression_end(&text[..at as usize], ts)
    {
        // Without TypeScript `as` is no operator, so acorn stops at the
        // first one after a complete expression. oxc would read a cast
        // there.
        return Read::Expression { end, lookahead: at };
    }
    let alloc = Allocator::default();
    let errors = match oxc_parser::Parser::new(&alloc, text, source_type(ts)).parse_expression() {
        Ok(expression) => {
            let end = expression.span().end;
            return Read::Expression {
                end,
                lookahead: next_token_start(text, end),
            };
        }
        Err(errors) => errors,
    };
    let len = text.len() as u32;
    let Some(first) = errors.first() else {
        return Read::Error {
            at: len,
            message: "Unexpected token",
        };
    };
    let mut label = first.labels.first().map_or(len, |l| l.span().start);
    let mut message = first.message.as_ref();
    // A fatal error drops the tokenizer's errors from `parse_expression`;
    // one before it is the one acorn meets first.
    let earlier = tokenizer_error(text, ts).filter(|&(at, _)| at <= label);
    if let Some((at, tokenizer_message)) = earlier {
        label = at;
        message = tokenizer_message;
    }
    // Errors of acorn's tokenizer, thrown where the token starts.
    match message {
        "Unterminated string" if text[label as usize..].starts_with('`') => {
            let (at, message) = unterminated_template(text, label, ts);
            return Read::Error { at, message };
        }
        // oxc reports a template chunk that follows a `${…}` from the
        // `}` before it.
        "Unterminated string" if text[label as usize..].starts_with('}') => {
            let chunk = label + 1;
            return if chunk == len {
                Read::Error {
                    at: len,
                    message: "Unterminated template literal",
                }
            } else {
                Read::Error {
                    at: chunk,
                    message: "Unterminated template",
                }
            };
        }
        "Unterminated string" => {
            return Read::Error {
                at: label,
                message: "Unterminated string constant",
            };
        }
        "Unterminated multiline comment" => {
            return Read::Error {
                at: label,
                message: "Unterminated comment",
            };
        }
        _ => {}
    }
    let at = match message {
        // acorn throws at the start of the assignment's left side.
        "Cannot assign to this expression" => {
            return Read::Error {
                at: label,
                message: acorn_message(message),
            };
        }
        // Without TypeScript acorn meets the cast's `as` where it expects
        // the rest of the expression around it.
        CAST_WITHOUT_TYPESCRIPT => cast_keyword(text, label, ts).unwrap_or(label),
        _ => next_token_start(text, label).min(len),
    };
    match complete_expression_end(&text[..at as usize], ts) {
        Some(end) => Read::Expression { end, lookahead: at },
        None => Read::Error {
            at,
            message: acorn_message(message),
        },
    }
}

/// The first error oxc's tokenizer reports in `text` read as an
/// expression: where, and its message.
fn tokenizer_error(text: &str, ts: bool) -> Option<(u32, &'static str)> {
    if !text.contains(['"', '\'', '`', '/']) {
        return None;
    }
    let wrapped = format!("({text}");
    let alloc = Allocator::default();
    let parsed = oxc_parser::Parser::new(&alloc, &wrapped, source_type(ts)).parse();
    parsed.diagnostics.iter().find_map(|d| {
        let message = match d.message.as_ref() {
            "Unterminated string" => "Unterminated string",
            "Unterminated multiline comment" => "Unterminated multiline comment",
            _ => return None,
        };
        let at = d.labels.first()?.span().start.checked_sub(1)?;
        Some((at, message))
    })
}

/// Where acorn throws for the template literal that opens at `tick` in
/// `text` and does not close before the text ends, and its message.
///
/// acorn reads a template one chunk at a time. A chunk that the text
/// ends inside is `Unterminated template`, at the chunk's start. When
/// the text ends right after a `${…}`, there is no chunk to start, and
/// the parser throws `Unterminated template literal` at the end. An
/// error inside a `${…}` that does not close is that expression's own.
fn unterminated_template(text: &str, tick: u32, ts: bool) -> (u32, &'static str) {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut chunk = tick as usize + 1;
    let mut i = chunk;
    while i < len {
        match bytes[i] {
            b'\\' => i += 2,
            b'$' if bytes.get(i + 1) == Some(&b'{') => match match_bracket(text, i as u32 + 1) {
                Ok(end) => {
                    i = end as usize;
                    chunk = i;
                }
                Err(_) => {
                    let inner = i + 2;
                    return match read_expression(&text[inner..], ts) {
                        Read::Error { at, message } => (inner as u32 + at, message),
                        // acorn then expects the `}` and meets the end.
                        Read::Expression { .. } => (len as u32, "Unexpected token"),
                    };
                }
            },
            _ => i += 1,
        }
    }
    if chunk == len && chunk > tick as usize + 1 {
        (len as u32, "Unterminated template literal")
    } else {
        (chunk as u32, "Unterminated template")
    }
}

/// The first `as` or `satisfies` keyword in `text` from `from` that is
/// the next token after a complete expression starting there.
fn cast_keyword(text: &str, from: u32, ts: bool) -> Option<u32> {
    let rest = text.get(from as usize..)?;
    let is_word = |c: Option<char>| c.is_some_and(oxc_syntax::identifier::is_identifier_part);
    let mut found: Vec<(usize, usize)> = ["as", "satisfies"]
        .into_iter()
        .flat_map(|keyword| rest.match_indices(keyword).map(|(p, k)| (p, k.len())))
        .collect();
    found.sort_unstable();
    found.into_iter().find_map(|(p, keyword_len)| {
        let before = rest[..p].chars().next_back();
        let after = rest[p + keyword_len..].chars().next();
        if p == 0 || is_word(before) || is_word(after) {
            return None;
        }
        let end = complete_expression_end(&rest[..p], ts)?;
        // A keyword inside a comment after the expression is no token.
        (next_token_start(rest, end) == p as u32).then_some(from + p as u32)
    })
}

/// The comments acorn lexes in `text` before `to`, in order, as
/// `(start, end)` (strings and template literals hold no comments).
pub fn comments_before(text: &str, to: u32, ts: bool) -> Vec<(u32, u32)> {
    let Some(text) = text.get(..to as usize) else {
        return Vec::new();
    };
    if !text.contains("//") && !text.contains("/*") {
        return Vec::new();
    }
    // An opening parenthesis keeps the text in expression position; oxc
    // keeps the comments it lexed even when it then fails.
    let wrapped = format!("({text}");
    let alloc = Allocator::default();
    let parsed = oxc_parser::Parser::new(&alloc, &wrapped, source_type(ts)).parse();
    parsed
        .program
        .comments
        .iter()
        .map(|c| (c.span.start - 1, c.span.end - 1))
        .filter(|&(_, end)| end <= to)
        .collect()
}

/// The start of the first token at or after `at`: whitespace and
/// comments skipped.
pub fn next_token_start(text: &str, mut at: u32) -> u32 {
    loop {
        let Some(rest) = text.get(at as usize..) else {
            return at;
        };
        let trimmed = rest.trim_start();
        at += (rest.len() - trimmed.len()) as u32;
        if let Some(line) = trimmed.strip_prefix("//") {
            let len = line
                .find(['\n', '\r', '\u{2028}', '\u{2029}'])
                .unwrap_or(line.len());
            at += 2 + len as u32;
        } else if let Some(block) = trimmed.strip_prefix("/*") {
            match block.find("*/") {
                Some(end) => at += 2 + end as u32 + 2,
                None => return text.len() as u32,
            }
        } else {
            return at;
        }
    }
}

/// An error the compiler's `match_bracket()` throws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BracketError {
    /// `expected_token`: a closing bracket that is not the partner of
    /// the open one, at `at`.
    Mismatch { at: u32, expected: char },
    /// `unexpected_eof`, at the end of the text.
    Eof { at: u32 },
    /// `unterminated_string_constant`, at the start of the string's body.
    UnterminatedString { at: u32 },
}

/// `match_bracket()` from the `{` / `[` / `(` at `start` in `text`: the
/// position after its partner. Strings are skipped, comments are not.
pub fn match_bracket(text: &str, start: u32) -> Result<u32, BracketError> {
    match_brackets(text, start, &[(b'(', b')'), (b'[', b']'), (b'{', b'}')])
}

/// `match_bracket()` with its `brackets` argument: only these pairs
/// count (a snippet's type parameters use `<` / `>` alone).
pub fn match_brackets(text: &str, start: u32, pairs: &[(u8, u8)]) -> Result<u32, BracketError> {
    let bytes = text.as_bytes();
    let mut stack: Vec<u8> = Vec::new();
    let mut i = start as usize;
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        match c {
            b'\'' | b'"' | b'`' => i = match_quote(text, i, c)?,
            _ if pairs.iter().any(|&(open, _)| open == c) => stack.push(c),
            _ if pairs.iter().any(|&(_, close)| close == c) => {
                let open = stack.pop();
                let expected = pairs
                    .iter()
                    .find(|&&(o, _)| Some(o) == open)
                    .map_or(b'}', |&(_, close)| close);
                if c != expected {
                    return Err(BracketError::Mismatch {
                        at: i as u32 - 1,
                        expected: char::from(expected),
                    });
                }
                if stack.is_empty() {
                    return Ok(i as u32);
                }
            }
            _ => {}
        }
    }
    Err(BracketError::Eof {
        at: bytes.len() as u32,
    })
}

/// `match_quote()`: the position after the string whose body starts at
/// `start`.
fn match_quote(text: &str, start: usize, quote: u8) -> Result<usize, BracketError> {
    let bytes = text.as_bytes();
    let mut escaped = false;
    let mut i = start;
    while i < bytes.len() {
        let c = bytes[i];
        i += 1;
        if escaped {
            escaped = false;
            continue;
        }
        if c == quote {
            return Ok(i);
        }
        if c == b'\\' {
            escaped = true;
        }
        if quote == b'`' && c == b'$' && bytes.get(i) == Some(&b'{') {
            i = match_bracket(text, i as u32)? as usize;
        }
    }
    Err(BracketError::UnterminatedString { at: start as u32 })
}

/// The compiler's read of a pattern's type annotation (TypeScript only),
/// from the text after its colon.
pub struct AnnotationRead {
    /// What acorn reads: `_ as ` and the text after the colon, in which
    /// `?:` became `:` so that function parameters read as a sequence do
    /// not fail on optional ones. Positions in it after the first five
    /// map to the template as the compiler maps them, shifted by what
    /// that replacement removed.
    pub text: String,
    /// Where the annotation ends in `text`, or the error acorn throws.
    pub read: Read,
}

/// The length of the `_ as ` the compiler writes over the five characters
/// before a type annotation.
pub const ANNOTATION_PREFIX_LEN: u32 = 5;

pub fn read_type_annotation(after_colon: &str) -> AnnotationRead {
    let text = format!("_ as {}", drop_optional_marks(after_colon));
    let mut read = read_expression(&text, true);
    // `foo: bar = baz` reads as an assignment: read again up to its `=`.
    if let Read::Expression { end, .. } = read
        && let Some(eq) = assignment_operator(&text[..end as usize])
    {
        read = read_expression(&text[..eq as usize], true);
    }
    // `as item: string, index` reads `string, index` as a sequence: its
    // first expression is the annotation.
    if let Read::Expression { end, lookahead } = read
        && let Some(first_end) = sequence_first_end(&text[..end as usize])
    {
        read = Read::Expression {
            end: first_end,
            lookahead,
        };
    }
    AnnotationRead { text, read }
}

/// When `text` is an assignment, the position of its `=`.
fn assignment_operator(text: &str) -> Option<u32> {
    let alloc = Allocator::default();
    let expression = oxc_parser::Parser::new(&alloc, text, source_type(true))
        .parse_expression()
        .ok()?;
    let Expression::AssignmentExpression(assignment) = unparenthesized(&expression) else {
        return None;
    };
    let right = assignment.right.span().start as usize;
    text.get(..right)?.rfind('=').map(|eq| eq as u32)
}

/// When `text` is a sequence, where its first expression ends.
fn sequence_first_end(text: &str) -> Option<u32> {
    let alloc = Allocator::default();
    let expression = oxc_parser::Parser::new(&alloc, text, source_type(true))
        .parse_expression()
        .ok()?;
    match unparenthesized(&expression) {
        Expression::SequenceExpression(sequence) => Some(sequence.expressions.first()?.span().end),
        _ => None,
    }
}

/// The compiler's `.replace(/\?\s*:/g, ':')`.
fn drop_optional_marks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(q) = rest.find('?') {
        out.push_str(&rest[..q]);
        let after = &rest[q + 1..];
        let trimmed = after.trim_start_matches(char::is_whitespace);
        if trimmed.starts_with(':') {
            rest = trimmed;
        } else {
            out.push('?');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_where_the_expression_ends() {
        assert_eq!(
            read_expression("a b", false),
            Read::Expression {
                end: 1,
                lookahead: 2
            }
        );
        assert_eq!(
            read_expression("[] as q q", true),
            Read::Expression {
                end: 7,
                lookahead: 8
            }
        );
        // Without TypeScript, at the first `as`.
        assert_eq!(
            read_expression("[] as q", false),
            Read::Expression {
                end: 2,
                lookahead: 3
            }
        );
    }

    #[test]
    fn throws_inside_an_unfinished_expression() {
        assert_eq!(
            read_expression("[] as {a b}", true),
            Read::Error {
                at: 9,
                message: "Unexpected token"
            }
        );
        assert_eq!(
            read_expression("f(x as any) as q", false),
            Read::Error {
                at: 4,
                message: "Unexpected token"
            }
        );
        assert_eq!(
            read_expression("[] as q = 1", true),
            Read::Error {
                at: 0,
                message: "Assigning to rvalue"
            }
        );
    }

    #[test]
    fn tokenizer_errors_are_thrown_where_the_token_starts() {
        assert_eq!(
            read_expression("(/* ) => {}", false),
            Read::Error {
                at: 1,
                message: "Unterminated comment"
            }
        );
        assert_eq!(
            read_expression("(a = \") => {}", false),
            Read::Error {
                at: 5,
                message: "Unterminated string constant"
            }
        );
        assert_eq!(
            read_expression("(a = `) => {}", false),
            Read::Error {
                at: 6,
                message: "Unterminated template"
            }
        );
    }

    #[test]
    fn type_annotation_ends_with_its_type() {
        let end = |after: &str| match read_type_annotation(after).read {
            Read::Expression { end, .. } => Some(end - ANNOTATION_PREFIX_LEN),
            Read::Error { .. } => None,
        };
        assert_eq!(end(" Map<string, number>"), Some(20));
        assert_eq!(end(" (a: string) => void, i"), Some(20));
        assert_eq!(end(" string = 1"), Some(7));
        assert_eq!(end(" string string"), Some(7));
        assert_eq!(end(" "), None);
    }

    #[test]
    fn bracket_errors() {
        assert_eq!(
            match_bracket("{a]}", 0),
            Err(BracketError::Mismatch {
                at: 2,
                expected: '}'
            })
        );
        assert_eq!(match_bracket("[a, {b}] x", 0), Ok(8));
        assert_eq!(match_bracket("{a", 0), Err(BracketError::Eof { at: 2 }));
        assert_eq!(
            match_bracket("{'a}", 0),
            Err(BracketError::UnterminatedString { at: 2 })
        );
    }
}
