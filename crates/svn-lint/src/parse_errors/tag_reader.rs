//! The compiler's character-level reads inside a block tag.
//!
//! Most of a tag is JavaScript that the compiler hands to acorn, which
//! skips comments like any JavaScript parser. The rest is read by the
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
//! compiler throws there. Where the outcome depends on reads this
//! module does not model, it reports nothing.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, Statement};
use oxc_span::{GetSpan, SourceType};
use svn_core::Range;
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
}

/// How the compiler continues after a pattern, which decides how its
/// type annotation is delimited when we hand it to oxc.
#[derive(Clone, Copy)]
enum PatternSite {
    /// An `{#each}` context or a `then` / `catch` value: the pattern
    /// text alone, without an initializer.
    Binding,
    /// A `{@const}`: the pattern is followed by `= init`.
    Const,
}

/// A pattern the reader got past: where it ends, and where the `:` of
/// its type annotation sits when it has one.
struct Pattern {
    end: u32,
    annotation: Option<u32>,
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

    fn comment_end(&self, at: u32) -> Option<u32> {
        let rest = self.source.get(at as usize..)?;
        if rest.starts_with("//") {
            Some(at + rest.find('\n').unwrap_or(rest.len()) as u32)
        } else if let Some(body) = rest.strip_prefix("/*") {
            body.find("*/").map(|i| at + 2 + i as u32 + 2)
        } else {
            None
        }
    }

    fn skip_whitespace_and_comments(&self, mut at: u32) -> u32 {
        loop {
            at = self.skip_whitespace(at);
            match self.comment_end(at) {
                Some(end) => at = end,
                None => return at,
            }
        }
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

    /// `match_bracket()` from the `{` / `[` at `start`: the position
    /// after its partner. Strings are skipped, comments are not.
    /// `None` where the compiler would throw an error of its own.
    fn match_bracket(&self, start: u32) -> Option<u32> {
        let bytes = self.source.as_bytes();
        let mut stack: Vec<u8> = Vec::new();
        let mut i = start as usize;
        while i < bytes.len() {
            let c = bytes[i];
            i += 1;
            match c {
                b'\'' | b'"' | b'`' => i = self.match_quote(i, c)?,
                b'(' | b'[' | b'{' => stack.push(c),
                b')' | b']' | b'}' => {
                    let open = stack.pop()?;
                    let expected = match open {
                        b'(' => b')',
                        b'[' => b']',
                        _ => b'}',
                    };
                    if c != expected {
                        return None;
                    }
                    if stack.is_empty() {
                        return Some(i as u32);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// `match_quote()`: the position after the string whose body
    /// starts at `i`.
    fn match_quote(&self, mut i: usize, quote: u8) -> Option<usize> {
        let bytes = self.source.as_bytes();
        let mut escaped = false;
        while i < bytes.len() {
            let c = bytes[i];
            i += 1;
            if escaped {
                escaped = false;
                continue;
            }
            if c == quote {
                return Some(i);
            }
            if c == b'\\' {
                escaped = true;
            }
            if quote == b'`' && c == b'$' && bytes.get(i) == Some(&b'{') {
                i = self.match_bracket(i as u32)? as usize;
            }
        }
        None
    }

    /// The `)` closing the `(` at `open`, read as JavaScript (strings
    /// and comments skipped), and whether a comment lies between them.
    fn matching_paren(&self, open: u32) -> Option<(u32, bool)> {
        let bytes = self.source.as_bytes();
        let mut depth = 0u32;
        let mut has_comment = false;
        let mut i = open as usize;
        while i < bytes.len() {
            match bytes[i] {
                b'\'' | b'"' | b'`' => {
                    i = self.match_quote(i + 1, bytes[i])?;
                    continue;
                }
                b'/' => {
                    if let Some(end) = self.comment_end(i as u32) {
                        has_comment = true;
                        i = end as usize;
                        continue;
                    }
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth = depth.checked_sub(1)?;
                    if depth == 0 {
                        return (bytes[i] == b')').then_some((i as u32, has_comment));
                    }
                }
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// `read_pattern()` from `start`, followed by its optional type
    /// annotation. `region_end` bounds the text our parser read as the
    /// pattern's part of the tag.
    fn read_pattern(
        &self,
        start: u32,
        region_end: u32,
        site: PatternSite,
    ) -> Result<Option<Pattern>, TagError> {
        let first = self.char_at(start);
        let end = if first.is_some_and(oxc_syntax::identifier::is_identifier_start) {
            self.identifier_end(start)
        } else if matches!(first, Some('{' | '[')) {
            match self.match_bracket(start) {
                Some(end) => end,
                None => return Ok(None),
            }
        } else {
            return Err(TagError::expected_pattern(start));
        };
        let colon = self.skip_whitespace(end);
        if self.byte(colon) != Some(b':') {
            return Ok(Some(Pattern {
                end,
                annotation: None,
            }));
        }
        // The compiler reads the annotation by handing acorn the text
        // from five characters before the type, with `_ as ` written
        // over them. Without TypeScript acorn reads only the `_`, and the
        // compiler carries on reading right after it.
        if !self.ts {
            return Ok(colon.checked_sub(3).map(|end| Pattern {
                end,
                annotation: None,
            }));
        }
        let Some(end) = self.annotated_pattern_end(start, region_end, site) else {
            return Ok(None);
        };
        Ok(Some(Pattern {
            end,
            annotation: Some(colon),
        }))
    }

    /// Where the type annotation of the pattern at `start` ends, read
    /// as TypeScript does (trailing comments are not part of it).
    fn annotated_pattern_end(&self, start: u32, region_end: u32, site: PatternSite) -> Option<u32> {
        let text = self.source.get(start as usize..region_end as usize)?;
        let (prefix, wrapped) = match site {
            PatternSite::Binding => ("let ", format!("let {text} = 0;")),
            PatternSite::Const => ("const ", format!("const {text};")),
        };
        let alloc = Allocator::default();
        let parsed = oxc_parser::Parser::new(&alloc, &wrapped, SourceType::ts()).parse();
        if !parsed.diagnostics.is_empty() || parsed.fatal_error {
            return None;
        }
        let Some(Statement::VariableDeclaration(decl)) = parsed.program.body.first() else {
            return None;
        };
        let annotation = decl.declarations.first()?.type_annotation.as_ref()?;
        Some(start + annotation.span.end - prefix.len() as u32)
    }

    /// Whether `text` reads as one complete expression.
    fn parses_as_expression(&self, text: &str) -> bool {
        let alloc = Allocator::default();
        let source_type = SourceType::default()
            .with_module(true)
            .with_typescript(self.ts);
        oxc_parser::Parser::new(&alloc, text, source_type)
            .parse_expression()
            .is_ok()
    }

    /// `{#each …}`.
    pub(super) fn each(&self, block: &EachBlock) -> Option<TagError> {
        let keyword_end = self.keyword_end(block.range.start, b'#', "each")?;
        if let Err(e) = self.require_whitespace(keyword_end) {
            return Some(e);
        }
        let expression_start = block.expression_range.start;
        match block.as_clause.as_ref().and_then(|c| c.context_range) {
            Some(context) => self.each_context(expression_start, context.start, Some(context.end)),
            None => self.each_without_context(block),
        }
    }

    /// An `{#each}` header in which our parser found no `as` context:
    /// either it has none (`{#each items, i}`), or the compiler finds
    /// one our parser did not (a TypeScript header).
    fn each_without_context(&self, block: &EachBlock) -> Option<TagError> {
        let range = block.expression_range;
        let text = self.source.get(range.start as usize..range.end as usize)?;
        let alloc = Allocator::default();
        let source_type = SourceType::default()
            .with_module(true)
            .with_typescript(self.ts);
        let Ok(expression) = oxc_parser::Parser::new(&alloc, text, source_type).parse_expression()
        else {
            // JavaScript has no `as`: the compiler's expression ends
            // where one does, and the context follows it.
            if self.ts {
                return None;
            }
            let at = self.as_after_javascript_expression(range.start, text)?;
            let context_start = match self.require_whitespace(at + 2) {
                Ok(start) => start,
                Err(e) => return Some(e),
            };
            return self.each_context(range.start, context_start, None);
        };
        // The compiler reads the header as one expression and, with no
        // `as` after it, keeps the first element of a sequence.
        let mut first = unparenthesized(&expression);
        if let Expression::SequenceExpression(sequence) = first {
            first = unparenthesized(sequence.expressions.first()?);
        }
        if let Expression::TSAsExpression(cast) = first {
            // A trailing `as` cast is the context: the compiler walks
            // back from the cast's type to its `as`.
            let mut at = (range.start + cast.type_annotation.span().start).checked_sub(2)?;
            while self.source.get(at as usize..at as usize + 2)? != "as" {
                at = at.checked_sub(1).filter(|&a| a > range.start)?;
            }
            let context_start = match self.require_whitespace(at + 2) {
                Ok(start) => start,
                Err(e) => return Some(e),
            };
            return self.each_context(range.start, context_start, None);
        }
        let at = self.skip_whitespace(range.start + first.span().end);
        self.each_index_and_key(range.start, at, None)
    }

    /// The first `as` in the header `text` (starting at `start`) that
    /// follows a complete JavaScript expression: where acorn stops
    /// reading the iterable and the compiler looks for `as`.
    fn as_after_javascript_expression(&self, start: u32, text: &str) -> Option<u32> {
        text.match_indices("as").find_map(|(p, _)| {
            let in_identifier = text[..p]
                .chars()
                .next_back()
                .is_some_and(oxc_syntax::identifier::is_identifier_part);
            (!in_identifier && self.parses_as_expression(&text[..p])).then_some(start + p as u32)
        })
    }

    /// The `as` context of an `{#each}` and what follows it.
    fn each_context(
        &self,
        expression_start: u32,
        context_start: u32,
        context_end: Option<u32>,
    ) -> Option<TagError> {
        // Without our parser's bound for the pattern, an annotated one
        // is not followed.
        let region_end = context_end.unwrap_or(context_start);
        let pattern = match self.read_pattern(context_start, region_end, PatternSite::Binding) {
            Ok(Some(pattern)) => pattern,
            Ok(None) => return None,
            Err(e) => return Some(e),
        };
        let at = self.skip_whitespace(pattern.end);
        self.each_index_and_key(expression_start, at, pattern.annotation)
    }

    /// From `at`, after the iterable or the context: `, index`, then
    /// `(key)`, then the closing `}`.
    fn each_index_and_key(
        &self,
        expression_start: u32,
        mut at: u32,
        annotation: Option<u32>,
    ) -> Option<TagError> {
        if self.byte(at) == Some(b',') {
            at = self.skip_whitespace(at + 1);
            let end = self.identifier_end(at);
            if end == at {
                return Some(TagError::expected_identifier(at));
            }
            at = self.skip_whitespace(end);
        }
        if self.byte(at) == Some(b'(') {
            let (close, key_has_comment) = self.matching_paren(at)?;
            let after = self.skip_whitespace(close + 1);
            if self.byte(after) == Some(b'}') {
                return None;
            }
            if self.comment_end(after).is_some()
                && !key_has_comment
                && self.read_past_key(expression_start, annotation, close)
            {
                // A comment after the key was already collected by an
                // earlier acorn read that ran through the key. Reading
                // the key, the compiler moves past the last comment
                // collected so far, so it looks for the `)` there.
                return Some(TagError::expected_token(
                    self.skip_whitespace_and_comments(after),
                    ")",
                ));
            }
            return Some(TagError::expected_token(after, "}"));
        }
        (self.byte(at) != Some(b'}')).then(|| TagError::expected_token(at, "}"))
    }

    /// Whether one of the acorn reads before the key's ran through the
    /// key's `)` at `close`, collecting the comments after it: the
    /// first read of the header as an expression (`items, i (key)` is a
    /// call; in TypeScript so is `items as item, i (key)`), or, in
    /// TypeScript, the read of the context's type annotation, which the
    /// compiler parses as `_ as Type, i (key)`.
    fn read_past_key(&self, expression_start: u32, annotation: Option<u32>, close: u32) -> bool {
        let through = |from: u32| self.source.get(from as usize..close as usize + 1);
        if through(expression_start).is_some_and(|t| self.parses_as_expression(t)) {
            return true;
        }
        self.ts
            && annotation
                .and_then(|colon| through(colon + 1))
                .is_some_and(|t| self.parses_as_expression(&format!("_ as {t}")))
    }

    /// The value of a `then` / `catch` (inline in `{#await}` or in a
    /// `{:then}` / `{:catch}` tag), as our parser delimited it: the
    /// text between the keyword's whitespace and the closing `}`.
    pub(super) fn branch_value(&self, value: Range) -> Option<TagError> {
        if let Some(e) = self.whitespace_before_part(value.start) {
            return Some(e);
        }
        let pattern = match self.read_pattern(value.start, value.end, PatternSite::Binding) {
            Ok(Some(pattern)) => pattern,
            Ok(None) => return None,
            Err(e) => return Some(e),
        };
        let at = self.skip_whitespace(pattern.end);
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

    /// `{@const pattern = init}`.
    pub(super) fn const_tag(&self, tag: &Interpolation) -> Option<TagError> {
        let keyword_end = self.keyword_end(tag.range.start, b'@', "const")?;
        let start = match self.require_whitespace(keyword_end) {
            Ok(start) => start,
            Err(e) => return Some(e),
        };
        let region_end = tag.expression_range.end.max(start);
        let pattern = match self.read_pattern(start, region_end, PatternSite::Const) {
            Ok(Some(pattern)) => pattern,
            Ok(None) => return None,
            Err(e) => return Some(e),
        };
        let at = self.skip_whitespace(pattern.end);
        (self.byte(at) != Some(b'=')).then(|| TagError::expected_token(at, "="))
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
