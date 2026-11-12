//! The compiler's reads inside a tag.
//!
//! The compiler parses a template in one pass. At an opening tag it reads
//! the tag's parts in order: the whitespace after its keyword, its
//! expression (handed to acorn, see [`crate::acorn`]), the pattern after
//! an `{#each … as` or a `then` / `catch` / `{@const` (with its own
//! bracket matcher, then checked by acorn), `, index`, `(key)`, and the
//! `}` it then expects. The tag ends wherever that reading ends: there is
//! no separate step that finds the `}` first. The character-level parts
//! skip whitespace only, so a comment where a token is expected is that
//! token's error, at the comment.
//!
//! [`TagReader`] replays those reads over the source. The parser ends
//! and splits every tag with them, and keeps the error of a reading that
//! fails on the node, where the lint reports it from.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, TSAsExpression};
use oxc_ast_visit::{Visit, walk};
use oxc_span::{GetSpan, SourceType};
use svn_core::Range;

use crate::acorn::{self, BracketError, Read};

/// An error the compiler throws while reading a tag, at `at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagReadError {
    pub at: u32,
    pub kind: TagReadErrorKind,
    /// The text acorn was reading when it threw, for a pattern or a
    /// snippet's parameters. acorn has errors of its own for these
    /// (a keyword as a shorthand name, a comma after a rest element);
    /// the lint works out which one from this.
    pub read: Option<AcornRead>,
}

/// A part of the template the compiler wraps before acorn reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcornRead {
    pub range: Range,
    pub shape: AcornReadShape,
}

/// How the compiler wraps an [`AcornRead`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcornReadShape {
    /// A destructuring pattern, read as `(pattern = 1)`.
    Pattern,
    /// A snippet's type parameters and parenthesized parameters, read
    /// as `parameters => {}`.
    Parameters,
    /// An expression, read where it stands. To find acorn's error for
    /// it the lint reads it as the statement `_ = expression`: the right
    /// side of an assignment is read exactly as an expression on its own
    /// (parentheses would turn `a,` into arrow parameters, and nothing
    /// in front would turn `{ a }` into a block).
    Expression,
}

impl AcornRead {
    /// The text acorn reads for this part of `source`, and the offset in
    /// `source` its first character stands for.
    pub fn wrapped(&self, source: &str) -> (String, u32) {
        let text = self.range.slice(source);
        match self.shape {
            AcornReadShape::Pattern => {
                (format!("({text} = 1)"), self.range.start.saturating_sub(1))
            }
            AcornReadShape::Parameters => (format!("{text} => {{}}"), self.range.start),
            AcornReadShape::Expression => {
                (format!("_ = {text}"), self.range.start.saturating_sub(4))
            }
        }
    }
}

/// The compiler error a [`TagReadError`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagReadErrorKind {
    /// `expected_token` for the token.
    ExpectedToken(&'static str),
    ExpectedWhitespace,
    ExpectedPattern,
    ExpectedIdentifier,
    /// `unexpected_reserved_word` for the word.
    ReservedWord(String),
    UnexpectedEof,
    UnterminatedStringConstant,
    /// `js_parse_error` with acorn's message.
    JsParse(&'static str),
}

impl TagReadError {
    fn new(at: u32, kind: TagReadErrorKind) -> Self {
        Self {
            at,
            kind,
            read: None,
        }
    }

    fn reading(mut self, read: AcornRead) -> Self {
        self.read = Some(read);
        self
    }

    /// The error `match_bracket()` throws.
    fn bracket(error: BracketError) -> Self {
        match error {
            BracketError::Mismatch { at, expected } => Self::new(
                at,
                TagReadErrorKind::ExpectedToken(match expected {
                    ')' => ")",
                    ']' => "]",
                    _ => "}",
                }),
            ),
            BracketError::Eof { at } => Self::new(at, TagReadErrorKind::UnexpectedEof),
            BracketError::UnterminatedString { at } => {
                Self::new(at, TagReadErrorKind::UnterminatedStringConstant)
            }
        }
    }
}

/// An `{#each}` opening tag as the compiler read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EachOpen {
    /// The iterable.
    pub expression: Range,
    /// The `as` pattern, without its type annotation.
    pub context: Option<Range>,
    pub index: Option<Range>,
    /// The key expression inside its parentheses.
    pub key: Option<Range>,
    /// Just past the `}` that closes the tag.
    pub end: u32,
    /// The comments the compiler's reads collected, in source order.
    pub comments: Vec<Range>,
}

/// Which branch an `{#await}` opening tag starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AwaitBranch {
    Pending,
    /// `then`, with its value.
    Then(Option<Range>),
    /// `catch`, with its error.
    Catch(Option<Range>),
}

/// An `{#await}` opening tag as the compiler read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitOpen {
    pub expression: Range,
    pub branch: AwaitBranch,
    /// Just past the `}` that closes the tag.
    pub end: u32,
}

/// A `{:then}` / `{:catch}` tag as the compiler read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchOpen {
    pub value: Option<Range>,
    /// Just past the `}` that closes the tag.
    pub end: u32,
}

/// A `{#snippet}` opening tag as the compiler read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetOpen {
    pub name: Range,
    /// The type parameters inside their `<>`.
    pub generics: Option<Range>,
    /// The parameters inside their parentheses.
    pub parameters: Range,
    /// Just past the `}` that closes the tag.
    pub end: u32,
}

/// The comments acorn's reads in one tag collected.
#[derive(Debug, Default)]
pub struct Comments {
    /// Where the last one pushed ends: reading an expression moves past
    /// it.
    last_end: u32,
    /// Each one once.
    all: Vec<Range>,
}

/// Whether the compiler reads the component's JavaScript as TypeScript:
/// its `regex_lang_attribute` (`phases/1-parse/index.js`) finds the first
/// `<script` with whitespace after it, outside HTML comments, whose last
/// `lang=` attribute before the tag's `>` is exactly `ts`.
pub fn compiler_reads_typescript(source: &str) -> bool {
    let mut at = 0;
    while let Some(rel) = source[at..].find('<') {
        let start = at + rel;
        let rest = &source[start..];
        if rest.starts_with("<!--")
            && let Some(close) = rest[4..].find("-->")
        {
            at = start + 4 + close + 3;
            continue;
        }
        if let Some(lang) = script_lang_attribute(rest) {
            return lang == "ts";
        }
        at = start + 1;
    }
    false
}

/// The value the compiler's regex captures for a `<script` tag at the
/// start of `tag`: the last `lang=value` before its `>` that the regex
/// accepts. `None` when it matches nothing there.
fn script_lang_attribute(tag: &str) -> Option<&str> {
    let after = tag.strip_prefix("<script")?;
    if !after.starts_with(|c: char| c.is_whitespace()) {
        return None;
    }
    let attributes = &after[..after.find('>')?];
    attributes.rmatch_indices("lang=").find_map(|(p, _)| {
        // At least one whitespace character separates it from `<script`.
        if p == 0 {
            return None;
        }
        let value = &attributes[p + 5..];
        let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''));
        let value = &value[quote.map_or(0, char::len_utf8)..];
        let len = value.find(['"', '\'', ' ', '>']).unwrap_or(value.len());
        if len == 0 {
            return None;
        }
        if let Some(q) = quote
            && !value[len..].starts_with(q)
        {
            return None;
        }
        Some(&value[..len])
    })
}

/// `RESERVED_WORDS` from the compiler's `utils.js`.
pub const RESERVED_WORDS: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Replays the compiler's reads over `source`, the whole component.
pub struct TagReader<'s> {
    source: &'s str,
    ts: bool,
}

impl<'s> TagReader<'s> {
    /// `ts`: the compiler reads the component's JavaScript as TypeScript
    /// ([`compiler_reads_typescript`]).
    pub fn new(source: &'s str, ts: bool) -> Self {
        Self { source, ts }
    }

    pub fn byte(&self, at: u32) -> Option<u8> {
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
    pub fn skip_whitespace(&self, mut at: u32) -> u32 {
        loop {
            let len = self.whitespace_len(at);
            if len == 0 {
                return at;
            }
            at += len;
        }
    }

    /// Whether the character ending at `at` is whitespace.
    pub fn whitespace_before(&self, at: u32) -> bool {
        self.source
            .get(..at as usize)
            .and_then(|s| s.chars().next_back())
            .is_some_and(is_compiler_whitespace)
    }

    /// `require_whitespace()`: the position after the whitespace run, or
    /// the error at `at`.
    pub fn require_whitespace(&self, at: u32) -> Result<u32, TagReadError> {
        if self.whitespace_len(at) == 0 {
            return Err(TagReadError::new(at, TagReadErrorKind::ExpectedWhitespace));
        }
        Ok(self.skip_whitespace(at))
    }

    /// The end of the identifier at `at` (`at` itself when none starts
    /// there).
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

    /// `read_identifier()` at `at`: its end (`at` when none starts
    /// there), or the error for a reserved word.
    pub fn read_identifier(&self, at: u32) -> Result<u32, TagReadError> {
        let end = self.identifier_end(at);
        let name = &self.source[at as usize..end as usize];
        if RESERVED_WORDS.contains(&name) {
            return Err(TagReadError::new(
                at,
                TagReadErrorKind::ReservedWord(name.to_string()),
            ));
        }
        Ok(end)
    }

    /// acorn's read of an expression at `start`, of the template up to
    /// `limit`; the comments it collects are recorded in `comments`.
    fn read_expression(
        &self,
        start: u32,
        limit: u32,
        comments: &mut Comments,
    ) -> Result<u32, TagReadError> {
        let text = self
            .source
            .get(start as usize..limit as usize)
            .unwrap_or("");
        match acorn::read_expression(text, self.ts) {
            Read::Expression { end, lookahead } => {
                self.collect_comments(text, start, lookahead, comments);
                Ok(start + end)
            }
            Read::Error { at, message } => {
                self.collect_comments(text, start, at, comments);
                Err(
                    TagReadError::new(start + at, TagReadErrorKind::JsParse(message)).reading(
                        AcornRead {
                            range: Range::new(start, start + text.len() as u32),
                            shape: AcornReadShape::Expression,
                        },
                    ),
                )
            }
        }
    }

    /// Records the comments acorn collected reading `text` (which starts
    /// at `origin` in the template) up to `to` in it. Every read adds the
    /// comments it lexes to the compiler's list again, so the last
    /// comment is the last one the latest read lexed, wherever it lies.
    fn collect_comments(&self, text: &str, origin: u32, to: u32, comments: &mut Comments) {
        for (start, end) in acorn::comments_before(text, to, self.ts) {
            let range = Range::new(origin + start, origin + end);
            if !comments.all.contains(&range) {
                comments.all.push(range);
            }
            comments.last_end = range.end;
        }
    }

    /// `read_pattern()` from `start`, reading no further than `limit`:
    /// where the pattern ends (the context without its annotation), and
    /// where the reading carries on after its type annotation.
    pub fn read_pattern(
        &self,
        start: u32,
        limit: u32,
        comments: &mut Comments,
    ) -> Result<(u32, u32), TagReadError> {
        let identifier_end = self.read_identifier(start)?;
        let end = if identifier_end > start {
            identifier_end
        } else {
            if !matches!(self.byte(start), Some(b'{' | b'[')) {
                return Err(TagReadError::new(start, TagReadErrorKind::ExpectedPattern));
            }
            let end = acorn::match_bracket(self.source, start).map_err(TagReadError::bracket)?;
            // acorn reads the pattern as the left side of an assignment,
            // written where it stands in the template.
            let read = AcornRead {
                range: Range::new(start, end),
                shape: AcornReadShape::Pattern,
            };
            let (wrapped, origin) = read.wrapped(self.source);
            match acorn::read_expression(&wrapped, self.ts) {
                Read::Error { at, message } => {
                    return Err(
                        TagReadError::new(origin + at, TagReadErrorKind::JsParse(message))
                            .reading(read),
                    );
                }
                Read::Expression { lookahead, .. } => {
                    self.collect_comments(&wrapped, origin, lookahead, comments);
                }
            }
            end
        };
        let after = self.read_type_annotation(end, limit, comments)?;
        Ok((end, after))
    }

    /// `read_type_annotation()` after a pattern ending at `at`: where the
    /// reading carries on.
    fn read_type_annotation(
        &self,
        at: u32,
        limit: u32,
        comments: &mut Comments,
    ) -> Result<u32, TagReadError> {
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
            .get(after as usize..limit as usize)
            .unwrap_or("");
        let annotation = acorn::read_type_annotation(rest);
        match annotation.read {
            Read::Error { at, message } => Err(TagReadError::new(
                origin + at,
                TagReadErrorKind::JsParse(message),
            )),
            Read::Expression { end, lookahead } => {
                self.collect_comments(&annotation.text, origin, lookahead, comments);
                Ok(origin + end)
            }
        }
    }

    /// The `{#each}` opening tag whose keyword ends at `keyword_end`,
    /// reading no further than `limit` (where acorn would stop anyway:
    /// the `}` our brace count found, or the end of the fragment).
    pub fn read_each_open(&self, keyword_end: u32, limit: u32) -> Result<EachOpen, TagReadError> {
        let start = self.require_whitespace(keyword_end)?;
        let mut comments = Comments::default();

        // The iterable. When acorn throws, the compiler hides the
        // template from the last `as` before the error onwards and reads
        // again: `{#each x as { y = z }}` reads the pattern as an object
        // first.
        let mut cut = limit;
        let iterable_end = loop {
            match self.read_expression(start, cut, &mut comments) {
                Ok(end) => break end,
                Err(error) => match self.as_before(start, error.at) {
                    Some(at) => cut = at,
                    None => return Err(error),
                },
            }
        };

        // Reading an expression moves past the last collected comment.
        let mut index = self.skip_whitespace(iterable_end.max(comments.last_end));
        let mut expression_end = iterable_end;
        if !self.source[index as usize..].starts_with("as") {
            // No `as` after the iterable: it keeps the first expression
            // of a sequence, and a TypeScript cast that ends it is the
            // context.
            if let Some((end, cast)) = self.iterable_without_as(start, iterable_end) {
                expression_end = end;
                if let Some((inner_end, as_at)) = cast {
                    expression_end = inner_end;
                    index = as_at;
                }
            }
        }
        let (context, context_end) = if self.source[index as usize..].starts_with("as") {
            let context = self.require_whitespace(index + 2)?;
            let (pattern_end, end) = self.read_pattern(context, limit, &mut comments)?;
            (Some(Range::new(context, pattern_end)), end)
        } else {
            (None, expression_end)
        };

        let mut index = self.skip_whitespace(context_end);
        let mut index_range = None;
        if self.byte(index) == Some(b',') {
            index = self.skip_whitespace(index + 1);
            let end = self.read_identifier(index)?;
            if end == index {
                return Err(TagReadError::new(
                    index,
                    TagReadErrorKind::ExpectedIdentifier,
                ));
            }
            index_range = Some(Range::new(index, end));
            index = self.skip_whitespace(end);
        }
        let mut key = None;
        if self.byte(index) == Some(b'(') {
            let key_start = self.skip_whitespace(index + 1);
            let key_end = self.read_expression(key_start, limit, &mut comments)?;
            key = Some(Range::new(key_start, key_end));
            index = self.skip_whitespace(key_end.max(comments.last_end));
            if self.byte(index) != Some(b')') {
                return Err(TagReadError::new(
                    index,
                    TagReadErrorKind::ExpectedToken(")"),
                ));
            }
            index = self.skip_whitespace(index + 1);
        }
        if self.byte(index) != Some(b'}') {
            return Err(TagReadError::new(
                index,
                TagReadErrorKind::ExpectedToken("}"),
            ));
        }
        let mut comments = comments.all;
        comments.sort_by_key(|c| c.start);
        Ok(EachOpen {
            expression: Range::new(start, expression_end),
            context,
            index: index_range,
            key,
            end: index + 1,
            comments,
        })
    }

    /// The `}` the compiler requires at `at`, after whitespace: just past
    /// it.
    fn closing_brace(&self, at: u32) -> Result<u32, TagReadError> {
        let mut next = self.skip_whitespace(at);
        if next as usize >= self.source.len() {
            // The compiler reads a template whose end is trimmed, so
            // there is no whitespace to skip at the end of it.
            next = (self.source.trim_end().len() as u32).max(at);
        }
        let at = next;
        if self.byte(at) != Some(b'}') {
            return Err(TagReadError::new(at, TagReadErrorKind::ExpectedToken("}")));
        }
        Ok(at + 1)
    }

    /// A value after `then` / `catch` starting at `start` (whitespace
    /// required first): its range, the pattern with its annotation as
    /// svelte2tsx writes it, and where the reading carries on.
    fn branch_value(&self, start: u32, limit: u32) -> Result<(Range, u32), TagReadError> {
        let value = self.require_whitespace(start)?;
        let (pattern_end, end) = self.read_pattern(value, limit, &mut Comments::default())?;
        Ok((Range::new(value, pattern_end.max(end)), end))
    }

    /// The `{#await}` opening tag whose keyword ends at `keyword_end`.
    pub fn read_await_open(&self, keyword_end: u32, limit: u32) -> Result<AwaitOpen, TagReadError> {
        let start = self.require_whitespace(keyword_end)?;
        let mut comments = Comments::default();
        let expression_end = self.read_expression(start, limit, &mut comments)?;
        let index = self.skip_whitespace(expression_end.max(comments.last_end));
        let rest = &self.source[index as usize..];
        let keyword = if rest.starts_with("then") {
            Some("then")
        } else if rest.starts_with("catch") {
            Some("catch")
        } else {
            None
        };
        let (branch, index) = match keyword {
            None => (AwaitBranch::Pending, index),
            Some(keyword) => {
                let after = index + keyword.len() as u32;
                // `then}` / `then }`: the branch without a value.
                let (value, at) = if self.byte(self.skip_whitespace(after)) == Some(b'}') {
                    (None, after)
                } else {
                    let (value, end) = self.branch_value(after, limit)?;
                    (Some(value), end)
                };
                let branch = if keyword == "then" {
                    AwaitBranch::Then(value)
                } else {
                    AwaitBranch::Catch(value)
                };
                (branch, at)
            }
        };
        Ok(AwaitOpen {
            expression: Range::new(start, expression_end),
            branch,
            end: self.closing_brace(index)?,
        })
    }

    /// A `{:then}` / `{:catch}` tag whose keyword ends at `keyword_end`.
    pub fn read_branch(&self, keyword_end: u32, limit: u32) -> Result<BranchOpen, TagReadError> {
        if self.byte(keyword_end) == Some(b'}') {
            return Ok(BranchOpen {
                value: None,
                end: keyword_end + 1,
            });
        }
        let (value, end) = self.branch_value(keyword_end, limit)?;
        Ok(BranchOpen {
            value: Some(value),
            end: self.closing_brace(end)?,
        })
    }

    /// An expression read by acorn from `start`, then the `}` the
    /// compiler expects after it: just past that `}`.
    fn expression_then_brace(
        &self,
        start: u32,
        limit: u32,
        comments: &mut Comments,
    ) -> Result<u32, TagReadError> {
        let end = self.read_expression(start, limit, comments)?;
        // Reading an expression moves past the last collected comment.
        self.closing_brace(end.max(comments.last_end))
    }

    /// An `{expression}` tag whose `{` ends at `start`: just past the `}`
    /// that closes it.
    pub fn read_expression_tag(&self, start: u32, limit: u32) -> Result<u32, TagReadError> {
        self.expression_then_brace(start, limit, &mut Comments::default())
    }

    /// A tag that holds whitespace and one expression after its keyword
    /// (`{#if`, `{:else if`, `{#key`, `{@html`, `{@render`), whose
    /// keyword ends at `keyword_end`: just past the `}` that closes it.
    pub fn read_keyword_expression(
        &self,
        keyword_end: u32,
        limit: u32,
    ) -> Result<u32, TagReadError> {
        let start = self.require_whitespace(keyword_end)?;
        self.read_expression_tag(start, limit)
    }

    /// A `{@debug}` tag whose keyword ends at `keyword_end`. No
    /// whitespace is required after the keyword, and the tag may be
    /// empty.
    pub fn read_debug_tag(&self, keyword_end: u32, limit: u32) -> Result<u32, TagReadError> {
        let at = self.skip_whitespace(keyword_end);
        if self.byte(at) == Some(b'}') {
            return Ok(at + 1);
        }
        self.read_expression_tag(keyword_end, limit)
    }

    /// A `{@const pattern = expression}` tag whose keyword ends at
    /// `keyword_end`.
    pub fn read_const_tag(&self, keyword_end: u32, limit: u32) -> Result<u32, TagReadError> {
        let start = self.require_whitespace(keyword_end)?;
        let mut comments = Comments::default();
        let (_, after) = self.read_pattern(start, limit, &mut comments)?;
        let equals = self.skip_whitespace(after);
        if self.byte(equals) != Some(b'=') {
            return Err(TagReadError::new(
                equals,
                TagReadErrorKind::ExpectedToken("="),
            ));
        }
        let init = self.skip_whitespace(equals + 1);
        self.expression_then_brace(init, limit, &mut comments)
    }

    /// The `{#snippet}` opening tag whose keyword ends at `keyword_end`.
    /// Its parameter list ends at the first `)` that balances the `(`
    /// opening it, counted one character at a time (strings and
    /// comments too), and is then read by acorn as an arrow function's.
    pub fn read_snippet_open(&self, keyword_end: u32) -> Result<SnippetOpen, TagReadError> {
        let start = self.require_whitespace(keyword_end)?;
        let name_end = self.read_identifier(start)?;
        if name_end == start {
            return Err(TagReadError::new(
                start,
                TagReadErrorKind::ExpectedIdentifier,
            ));
        }
        let params_start = self.skip_whitespace(name_end);
        let mut index = params_start;
        let mut generics = None;
        if self.ts && self.byte(index) == Some(b'<') {
            let end = acorn::match_brackets(self.source, index, &[(b'<', b'>')])
                .map_err(TagReadError::bracket)?;
            generics = Some(Range::new(index + 1, end - 1));
            index = end;
        }
        index = self.skip_whitespace(index);
        if self.byte(index) != Some(b'(') {
            return Err(TagReadError::new(
                index,
                TagReadErrorKind::ExpectedToken("("),
            ));
        }
        let open = index;
        index += 1;
        // The compiler reads a template whose end is trimmed.
        let template_end = self.source.trim_end().len() as u32;
        let mut depth = 1;
        while index < template_end && !(self.byte(index) == Some(b')') && depth == 1) {
            match self.byte(index) {
                Some(b'(') => depth += 1,
                Some(b')') => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        if self.byte(index) != Some(b')') || index >= template_end {
            return Err(TagReadError::new(
                index,
                TagReadErrorKind::ExpectedToken(")"),
            ));
        }
        let close = index;
        let read = AcornRead {
            range: Range::new(params_start, close + 1),
            shape: AcornReadShape::Parameters,
        };
        let (wrapped, origin) = read.wrapped(self.source);
        if let Read::Error { at, message } = acorn::read_expression(&wrapped, self.ts) {
            return Err(
                TagReadError::new(origin + at, TagReadErrorKind::JsParse(message)).reading(read),
            );
        }
        Ok(SnippetOpen {
            name: Range::new(start, name_end),
            generics,
            parameters: Range::new(open + 1, close),
            end: self.closing_brace(close + 1)?,
        })
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
    /// them), and for a TypeScript cast that ends it, where the cast's
    /// expression ends and where its `as` is.
    fn iterable_without_as(&self, start: u32, end: u32) -> Option<(u32, Option<(u32, u32)>)> {
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
            found: None,
        };
        cast.visit_expression(first);
        let cast = cast.found.and_then(|(inner_end, type_start)| {
            // From just before the cast's type back to its `as` (a byte
            // offset inside a wider character is skipped over).
            let mut at = (start + type_start).checked_sub(2)?;
            while self.source.get(at as usize..at as usize + 2) != Some("as") {
                at = at.checked_sub(1).filter(|&a| a >= start)?;
            }
            Some((start + inner_end, at))
        });
        Some((start + first_end, cast))
    }
}

/// The outermost TypeScript cast that ends where the expression does
/// (the compiler's walk does not look inside the casts it matches): where
/// its expression ends and where its type starts.
struct TrailingCast {
    end: u32,
    found: Option<(u32, u32)>,
}

impl<'a> Visit<'a> for TrailingCast {
    fn visit_ts_as_expression(&mut self, it: &TSAsExpression<'a>) {
        if self.found.is_some() {
            return;
        }
        if it.span.end == self.end {
            self.found = Some((it.expression.span().end, it.type_annotation.span().start));
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
