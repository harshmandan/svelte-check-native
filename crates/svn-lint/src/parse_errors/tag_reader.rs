//! The compiler's reads inside a block tag.
//!
//! Most of a tag is JavaScript that the compiler hands to acorn
//! (acorn-typescript in a TypeScript component). The rest is read by the
//! compiler's own parser one character at a time (`state/tag.js`,
//! `read/context.js`): the whitespace a tag keyword must be followed
//! by, the pattern after an `{#each … as`, a `then` / `catch` or a
//! `{@const`, the `, index` and `(key)` of an `{#each}`, and the `}` /
//! `=` / `)` it expects between those parts. That reader skips
//! whitespace only, so a comment it meets where it wants a token is
//! reported as the token it wanted, at the comment's start.
//!
//! Each function replays those reads over the source, starting from
//! where our parser placed the tag, and returns the first error the
//! compiler throws there. acorn's reads are replayed with oxc (see
//! [`Read`]). acorn also adds every comment it passes to one list for
//! the whole component, and reading an expression moves on past the
//! last comment in that list, so the reads below keep the end of the
//! last comment acorn collected.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, TSAsExpression};
use oxc_ast_visit::{Visit, walk};
use oxc_span::{GetSpan, SourceType};
use svn_core::Range;
use svn_parser::acorn::{self, BracketError, Read};
use svn_parser::ast::{EachBlock, Interpolation};

use crate::codes::Code;
use crate::messages;

/// An error the compiler throws while reading a tag: where it throws,
/// its code and message. Every one of them is reported at a single
/// position.
pub(super) struct TagError {
    pub at: u32,
    pub code: Code,
    pub message: String,
}

impl TagError {
    fn expected_token(at: u32, token: &str) -> Self {
        Self {
            at,
            code: Code::expected_token,
            message: messages::expected_token(token),
        }
    }

    fn expected_whitespace(at: u32) -> Self {
        Self {
            at,
            code: Code::expected_whitespace,
            message: messages::expected_whitespace(),
        }
    }

    fn expected_pattern(at: u32) -> Self {
        Self {
            at,
            code: Code::expected_pattern,
            message: messages::expected_pattern(),
        }
    }

    fn expected_identifier(at: u32) -> Self {
        Self {
            at,
            code: Code::expected_identifier,
            message: messages::expected_identifier(),
        }
    }

    fn reserved_word(at: u32, word: &str) -> Self {
        Self {
            at,
            code: Code::unexpected_reserved_word,
            message: messages::unexpected_reserved_word(word),
        }
    }

    fn unexpected_eof(at: u32) -> Self {
        Self {
            at,
            code: Code::unexpected_eof,
            message: messages::unexpected_eof(),
        }
    }

    fn unterminated_string_constant(at: u32) -> Self {
        Self {
            at,
            code: Code::unterminated_string_constant,
            message: messages::unterminated_string_constant(),
        }
    }

    fn js_parse_error(at: u32, message: &str) -> Self {
        Self {
            at,
            code: Code::js_parse_error,
            message: format!("{message}\nhttps://svelte.dev/e/js_parse_error"),
        }
    }

    /// The error `match_bracket()` throws, in a text starting at
    /// `origin` in the template.
    fn bracket(origin: u32, error: BracketError) -> Self {
        match error {
            BracketError::Mismatch { at, expected } => {
                Self::expected_token(origin + at, &expected.to_string())
            }
            BracketError::Eof { at } => Self::unexpected_eof(origin + at),
            BracketError::UnterminatedString { at } => {
                Self::unterminated_string_constant(origin + at)
            }
        }
    }
}

pub(super) struct TagReader<'s> {
    source: &'s str,
    ts: bool,
}

impl<'s> TagReader<'s> {
    pub(super) fn new(source: &'s str, ts: bool) -> Self {
        Self { source, ts }
    }

    fn byte(&self, at: u32) -> Option<u8> {
        self.source.as_bytes().get(at as usize).copied()
    }

    fn char_at(&self, at: u32) -> Option<char> {
        self.source.get(at as usize..)?.chars().next()
    }

    /// Byte length of the compiler's whitespace character at `at`
    /// (`utils/whitespace.js`), or 0.
    fn whitespace_len(&self, at: u32) -> u32 {
        match self.char_at(at) {
            Some(c) if is_compiler_whitespace(c) => c.len_utf8() as u32,
            _ => 0,
        }
    }

    /// `allow_whitespace()`.
    fn skip_whitespace(&self, mut at: u32) -> u32 {
        loop {
            let len = self.whitespace_len(at);
            if len == 0 {
                return at;
            }
            at += len;
        }
    }

    /// Whether the character ending at `at` is whitespace.
    fn whitespace_before(&self, at: u32) -> bool {
        self.source
            .get(..at as usize)
            .and_then(|s| s.chars().next_back())
            .is_some_and(is_compiler_whitespace)
    }

    /// `require_whitespace()`: the position after the whitespace run,
    /// or the error at `at`.
    fn require_whitespace(&self, at: u32) -> Result<u32, TagError> {
        if self.whitespace_len(at) == 0 {
            return Err(TagError::expected_whitespace(at));
        }
        Ok(self.skip_whitespace(at))
    }

    /// `read_identifier()`: the end of the identifier at `at` (`at`
    /// itself when none starts there).
    fn identifier_end(&self, at: u32) -> u32 {
        let Some(rest) = self.source.get(at as usize..) else {
            return at;
        };
        let mut chars = rest.char_indices();
        match chars.next() {
            Some((_, c)) if oxc_syntax::identifier::is_identifier_start(c) => {}
            _ => return at,
        }
        for (i, c) in chars {
            if !oxc_syntax::identifier::is_identifier_part(c) {
                return at + i as u32;
            }
        }
        self.source.len() as u32
    }

    /// The end of the tag keyword `keyword` written after the tag's
    /// `{` and `sigil` (`#`, `@`), when the tag starts that way.
    fn keyword_end(&self, tag_start: u32, sigil: u8, keyword: &str) -> Option<u32> {
        let at = self.skip_whitespace(tag_start + 1);
        if self.byte(at) != Some(sigil) {
            return None;
        }
        let at = at + 1;
        self.source
            .get(at as usize..)?
            .starts_with(keyword)
            .then_some(at + keyword.len() as u32)
    }

    /// The whitespace that must follow a tag keyword (`{#if`, `{@html`,
    /// …).
    pub(super) fn keyword_whitespace(
        &self,
        tag_start: u32,
        sigil: u8,
        keyword: &str,
    ) -> Option<TagError> {
        let end = self.keyword_end(tag_start, sigil, keyword)?;
        self.require_whitespace(end).err()
    }

    /// The whitespace that must precede what our parser read as the
    /// part of a tag after its keyword (`{:else if cond}`, `{:then v}`),
    /// which starts right after that whitespace.
    pub(super) fn whitespace_before_part(&self, part_start: u32) -> Option<TagError> {
        (!self.whitespace_before(part_start)).then(|| TagError::expected_whitespace(part_start))
    }

    /// Records the comments acorn collected reading `text` (which starts
    /// at `origin` in the template) up to `to` in it. Each read adds the
    /// comments it lexes to the list again, so the last comment is the
    /// last one the latest read lexed, wherever it lies.
    fn collect_comments(&self, text: &str, origin: u32, to: u32, last_comment: &mut u32) {
        if let Some(end) = acorn::last_comment_end(text, to, self.ts) {
            *last_comment = origin + end;
        }
    }

    /// `read_identifier()` at `at`: its end (`at` when none starts
    /// there), or the error for a reserved word.
    fn read_identifier(&self, at: u32) -> Result<u32, TagError> {
        let end = self.identifier_end(at);
        let name = &self.source[at as usize..end as usize];
        if super::RESERVED_WORDS.contains(&name) {
            return Err(TagError::reserved_word(at, name));
        }
        Ok(end)
    }

    /// `read_pattern()` from `start`: where the pattern and its type
    /// annotation end. `close` is the `}` ending the tag, which no acorn
    /// read gets past.
    fn read_pattern(&self, start: u32, close: u32, comments: &mut u32) -> Result<u32, TagError> {
        let identifier_end = self.read_identifier(start)?;
        let end = if identifier_end > start {
            identifier_end
        } else {
            if !matches!(self.byte(start), Some(b'{' | b'[')) {
                return Err(TagError::expected_pattern(start));
            }
            let end =
                acorn::match_bracket(self.source, start).map_err(|e| TagError::bracket(0, e))?;
            // acorn reads the pattern as the left side of an assignment,
            // written where it stands in the template.
            let pattern = &self.source[start as usize..end as usize];
            let wrapped = format!("({pattern} = 1)");
            let origin = start - 1;
            match acorn::read_expression(&wrapped, self.ts) {
                Read::Error { at, message } => {
                    return Err(TagError::js_parse_error(origin + at, message));
                }
                Read::Expression { lookahead, .. } => {
                    self.collect_comments(&wrapped, origin, lookahead, comments);
                }
            }
            end
        };
        self.read_type_annotation(end, close, comments)
    }

    /// `read_type_annotation()` after a pattern ending at `at`: where
    /// the reading continues.
    fn read_type_annotation(
        &self,
        at: u32,
        close: u32,
        comments: &mut u32,
    ) -> Result<u32, TagError> {
        let colon = self.skip_whitespace(at);
        if self.byte(colon) != Some(b':') {
            return Ok(at);
        }
        // The compiler writes `_ as ` over the five characters before the
        // type and has acorn read an expression from there. Without
        // TypeScript acorn reads only the `_`, and the reading carries on
        // right after it.
        let after = colon + 1;
        let Some(origin) = after.checked_sub(acorn::ANNOTATION_PREFIX_LEN) else {
            return Ok(at);
        };
        if !self.ts {
            return Ok(origin + 1);
        }
        let rest = self
            .source
            .get(after as usize..close as usize)
            .unwrap_or("");
        let annotation = acorn::read_type_annotation(rest);
        match annotation.read {
            Read::Error { at, message } => Err(TagError::js_parse_error(origin + at, message)),
            Read::Expression { end, lookahead } => {
                self.collect_comments(&annotation.text, origin, lookahead, comments);
                Ok(origin + end)
            }
        }
    }

    /// `{#each …}`.
    pub(super) fn each(&self, block: &EachBlock) -> Option<TagError> {
        self.read_each(block).err()
    }

    fn read_each(&self, block: &EachBlock) -> Result<(), TagError> {
        let Some(keyword_end) = self.keyword_end(block.range.start, b'#', "each") else {
            return Ok(());
        };
        let start = self.require_whitespace(keyword_end)?;
        let Some(close) = block
            .opening_tag_end
            .checked_sub(1)
            .filter(|&c| self.byte(c) == Some(b'}'))
        else {
            return Ok(());
        };
        let mut comments = 0;

        // The iterable. When acorn throws, the compiler hides the
        // template from the last `as` before the error onwards and reads
        // again: `{#each x as { y = z }}` reads the pattern as an object
        // first.
        let mut limit = close;
        let iterable_end = loop {
            let text = &self.source[start as usize..limit as usize];
            match acorn::read_expression(text, self.ts) {
                Read::Expression { end, lookahead } => {
                    self.collect_comments(text, start, lookahead, &mut comments);
                    break start + end;
                }
                Read::Error { at, message } => {
                    self.collect_comments(text, start, at, &mut comments);
                    let at = start + at;
                    match self.as_before(start, at) {
                        Some(cut) => limit = cut,
                        None => return Err(TagError::js_parse_error(at, message)),
                    }
                }
            }
        };

        // Reading an expression moves past the last collected comment.
        let mut index = self.skip_whitespace(iterable_end.max(comments));
        let mut expression_end = iterable_end;
        if !self.source[index as usize..].starts_with("as") {
            // No `as` after the iterable: it keeps the first expression
            // of a sequence, and a TypeScript cast that ends it is the
            // context.
            if let Some((end, cast_as)) = self.iterable_without_as(start, iterable_end) {
                expression_end = end;
                if let Some(at) = cast_as {
                    index = at;
                }
            }
        }
        let context_end = if self.source[index as usize..].starts_with("as") {
            let context = self.require_whitespace(index + 2)?;
            self.read_pattern(context, close, &mut comments)?
        } else {
            expression_end
        };

        let mut index = self.skip_whitespace(context_end);
        if self.byte(index) == Some(b',') {
            index = self.skip_whitespace(index + 1);
            let end = self.read_identifier(index)?;
            if end == index {
                return Err(TagError::expected_identifier(index));
            }
            index = self.skip_whitespace(end);
        }
        if self.byte(index) == Some(b'(') {
            let key = self.skip_whitespace(index + 1);
            let text = &self.source[key as usize..close as usize];
            match acorn::read_expression(text, self.ts) {
                Read::Error { at, message } => {
                    return Err(TagError::js_parse_error(key + at, message));
                }
                Read::Expression { end, lookahead } => {
                    self.collect_comments(text, key, lookahead, &mut comments);
                    index = self.skip_whitespace((key + end).max(comments));
                }
            }
            if self.byte(index) != Some(b')') {
                return Err(TagError::expected_token(index, ")"));
            }
            index = self.skip_whitespace(index + 1);
        }
        if self.byte(index) != Some(b'}') {
            return Err(TagError::expected_token(index, "}"));
        }
        Ok(())
    }

    /// The last `as` that starts before `at - 1` and not before `start`.
    fn as_before(&self, start: u32, at: u32) -> Option<u32> {
        let mut cut = at.checked_sub(2)?;
        loop {
            if self.source.get(cut as usize..cut as usize + 2) == Some("as") {
                return Some(cut);
            }
            cut = cut.checked_sub(1).filter(|&c| c >= start)?;
        }
    }

    /// The iterable `start..end` when no `as` follows it: where its first
    /// expression ends (parentheses removed, as the compiler removes
    /// them), and the `as` of a TypeScript cast that ends it.
    fn iterable_without_as(&self, start: u32, end: u32) -> Option<(u32, Option<u32>)> {
        let text = self.source.get(start as usize..end as usize)?;
        let alloc = Allocator::default();
        let expression = oxc_parser::Parser::new(
            &alloc,
            text,
            SourceType::default()
                .with_module(true)
                .with_typescript(self.ts),
        )
        .parse_expression()
        .ok()?;
        let mut first = unparenthesized(&expression);
        if let Expression::SequenceExpression(sequence) = first {
            first = unparenthesized(sequence.expressions.first()?);
        }
        let first_end = first.span().end;
        let mut cast = TrailingCast {
            end: first_end,
            type_start: None,
        };
        cast.visit_expression(first);
        let cast_as = cast.type_start.and_then(|type_start| {
            // From two characters before the cast's type back to its `as`.
            let mut at = (start + type_start).checked_sub(2)?;
            while self.source.get(at as usize..at as usize + 2)? != "as" {
                at = at.checked_sub(1)?;
            }
            Some(at)
        });
        Some((start + first_end, cast_as))
    }

    /// The value of a `then` / `catch` (inline in `{#await}` or in a
    /// `{:then}` / `{:catch}` tag), as our parser delimited it: the
    /// text between the keyword's whitespace and the closing `}`.
    pub(super) fn branch_value(&self, value: Range) -> Option<TagError> {
        if let Some(e) = self.whitespace_before_part(value.start) {
            return Some(e);
        }
        let close = Some(self.skip_whitespace(value.end))
            .filter(|&c| self.byte(c) == Some(b'}'))
            .unwrap_or(value.end);
        let end = match self.read_pattern(value.start, close, &mut 0) {
            Ok(end) => end,
            Err(e) => return Some(e),
        };
        let at = self.skip_whitespace(end);
        (self.byte(at) != Some(b'}')).then(|| TagError::expected_token(at, "}"))
    }

    /// A `{:then}` / `{:catch}` tag our parser found no value in, whose
    /// body starts at `body_start`. Only a `}` right after the keyword
    /// ends the tag without a value; after whitespace the compiler
    /// reads a pattern, and finds the `}`.
    pub(super) fn missing_branch_value(&self, body_start: u32) -> Option<TagError> {
        let close = body_start.checked_sub(1)?;
        (self.byte(close) == Some(b'}') && self.whitespace_before(close))
            .then(|| TagError::expected_pattern(close))
    }

    /// `{@const pattern = init}`, up to its `=`.
    pub(super) fn const_tag(&self, tag: &Interpolation) -> Option<TagError> {
        let keyword_end = self.keyword_end(tag.range.start, b'@', "const")?;
        let start = match self.require_whitespace(keyword_end) {
            Ok(start) => start,
            Err(e) => return Some(e),
        };
        let close = tag.range.end.checked_sub(1)?;
        let end = match self.read_pattern(start, close, &mut 0) {
            Ok(end) => end,
            Err(e) => return Some(e),
        };
        let at = self.skip_whitespace(end);
        (self.byte(at) != Some(b'=')).then(|| TagError::expected_token(at, "="))
    }
}

/// The outermost TypeScript cast that ends where the expression does
/// (the compiler's walk does not look inside the casts it matches).
struct TrailingCast {
    end: u32,
    type_start: Option<u32>,
}

impl<'a> Visit<'a> for TrailingCast {
    fn visit_ts_as_expression(&mut self, it: &TSAsExpression<'a>) {
        if self.type_start.is_some() {
            return;
        }
        if it.span.end == self.end {
            self.type_start = Some(it.type_annotation.span().start);
            return;
        }
        walk::walk_ts_as_expression(self, it);
    }
}

fn unparenthesized<'a, 'b>(mut expr: &'a Expression<'b>) -> &'a Expression<'b> {
    while let Expression::ParenthesizedExpression(p) = expr {
        expr = &p.expression;
    }
    expr
}

/// The compiler's `is_whitespace`.
fn is_compiler_whitespace(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t'..='\r'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}'
    )
}
