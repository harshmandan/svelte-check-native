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
    /// A statement, read where it stands: a declaration tag's
    /// `const …` / `let …`.
    Statement,
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
            AcornReadShape::Statement => (text.to_string(), self.range.start),
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
    /// `attribute_empty_shorthand`: `{}` or `{@html …}` where an
    /// attribute is read.
    AttributeEmptyShorthand,
    ExpectedAttributeValue,
    /// `directive_missing_name` for the attribute name as written
    /// (`on:`). The error covers the name's prefix, up to `end`.
    DirectiveMissingName {
        name: String,
        end: u32,
    },
    DirectiveInvalidValue,
    /// `block_invalid_placement` for a `{#name …}` where only text and
    /// expressions may stand; `location` is how the compiler names the
    /// place (`in attribute value`, `inside <textarea>`).
    BlockInvalidPlacement {
        name: String,
        location: &'static str,
    },
    /// `tag_invalid_placement` for a `{@name …}` in such a place.
    TagInvalidPlacement {
        name: String,
        location: &'static str,
    },
    /// `js_parse_error` with acorn's message.
    JsParse(String),
}

impl TagReadError {
    pub(crate) fn new(at: u32, kind: TagReadErrorKind) -> Self {
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

/// A tag that ends with one expression, as the compiler read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpressionRead {
    /// Where acorn's expression ends. svelte2tsx copies the text up to
    /// here, so a comment or whitespace before the `}` is not part of it.
    pub expression_end: u32,
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
/// start of `tag`, or `None` when it matches nothing there.
///
/// The regex reads the tag's attributes two ways and takes the first
/// that fits. One runs to the tag's first `>` and takes the last
/// `lang=value` before it. The other steps over `name=value` attributes
/// one by one, where a quoted value may hold a `>`, and takes a `lang=`
/// that follows one directly.
fn script_lang_attribute(tag: &str) -> Option<&str> {
    let after = tag.strip_prefix("<script")?;
    let attributes = after.trim_start_matches(is_regex_space);
    if attributes.len() == after.len() {
        return None;
    }
    let to_first_close = &attributes[..attributes.find('>')?];
    to_first_close
        .rmatch_indices("lang=")
        .find_map(|(p, _)| lang_value(&attributes[p + 5..]))
        .or_else(|| lang_after_attributes(attributes))
}

/// What JavaScript's `\s` matches.
fn is_regex_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// `(["'])?([^"' >]+)\1[^>]*>` at the start of `text`: the value.
fn lang_value(text: &str) -> Option<&str> {
    let quote = text.chars().next().filter(|c| matches!(c, '"' | '\''));
    let value = &text[quote.map_or(0, char::len_utf8)..];
    let len = value.find(['"', '\'', ' ', '>']).unwrap_or(value.len());
    if len == 0 {
        return None;
    }
    let rest = &value[len..];
    let rest = match quote {
        Some(q) => rest.strip_prefix(q)?,
        None => rest,
    };
    rest.contains('>').then_some(&value[..len])
}

/// `(?:[^=>'"/]+=(?:"[^"]*"|'[^']*'|[^>\s]+)\s+)*lang=…` at the start
/// of `text`: as many attributes as will go, then the value of `lang=`.
fn lang_after_attributes(text: &str) -> Option<&str> {
    let name_len = text.find(['=', '>', '\'', '"', '/']).unwrap_or(text.len());
    let after_attribute = |value_len: usize| {
        let rest = &text[name_len + 1 + value_len..];
        let next = rest.trim_start_matches(is_regex_space);
        (next.len() < rest.len()).then_some(next)
    };
    if name_len > 0 && text[name_len..].starts_with('=') {
        let value = &text[name_len + 1..];
        let quoted = value
            .chars()
            .next()
            .filter(|c| matches!(c, '"' | '\''))
            .and_then(|q| value[1..].find(q))
            .map(|close| close + 2);
        let bare = value
            .find(|c: char| c == '>' || is_regex_space(c))
            .unwrap_or(value.len());
        let found = [quoted, Some(bare).filter(|&len| len > 0)]
            .into_iter()
            .flatten()
            .filter_map(after_attribute)
            .find_map(lang_after_attributes);
        if found.is_some() {
            return found;
        }
    }
    lang_value(text.strip_prefix("lang=")?)
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
            // One character past where the `_` was written. The compiler
            // counts those five characters in UTF-16 units.
            return Ok(self.units_before(after, acorn::ANNOTATION_PREFIX_LEN - 1));
        }
        let rest = self
            .source
            .get(after as usize..limit as usize)
            .unwrap_or("");
        let annotation = acorn::read_type_annotation(rest);
        match annotation.read {
            Read::Error { at, message } => Err(TagReadError::new(
                match acorn::ANNOTATION_PREFIX_LEN.checked_sub(at) {
                    // Inside the text written over the template.
                    Some(back) if back > 0 => self.units_before(after, back),
                    _ => origin + at,
                },
                TagReadErrorKind::JsParse(message),
            )),
            Read::Expression { end, lookahead } => {
                self.collect_comments(&annotation.text, origin, lookahead, comments);
                Ok(origin + end)
            }
        }
    }

    /// The offset `units` UTF-16 code units before `at`, or the start
    /// of the character that unit lies in.
    fn units_before(&self, at: u32, units: u32) -> u32 {
        let before = self.source.get(..at as usize).unwrap_or("");
        let mut left = units;
        for (i, c) in before.char_indices().rev() {
            if left == 0 {
                return (i + c.len_utf8()) as u32;
            }
            left = left.saturating_sub(c.len_utf16() as u32);
        }
        if left == 0 {
            0
        } else {
            at.saturating_sub(units)
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
    /// compiler expects after it.
    fn expression_then_brace(
        &self,
        start: u32,
        limit: u32,
        comments: &mut Comments,
    ) -> Result<ExpressionRead, TagReadError> {
        let expression_end = self.read_expression(start, limit, comments)?;
        // Reading an expression moves past the last collected comment.
        let end = self.closing_brace(expression_end.max(comments.last_end))?;
        Ok(ExpressionRead {
            expression_end,
            end,
        })
    }

    /// An `{expression}` tag whose `{` ends at `start`.
    pub fn read_expression_tag(
        &self,
        start: u32,
        limit: u32,
    ) -> Result<ExpressionRead, TagReadError> {
        self.expression_then_brace(start, limit, &mut Comments::default())
    }

    /// A tag that holds whitespace and one expression after its keyword
    /// (`{#if`, `{:else if`, `{#key`, `{@html`, `{@render`), whose
    /// keyword ends at `keyword_end`.
    pub fn read_keyword_expression(
        &self,
        keyword_end: u32,
        limit: u32,
    ) -> Result<ExpressionRead, TagReadError> {
        let start = self.require_whitespace(keyword_end)?;
        self.read_expression_tag(start, limit)
    }

    /// A `{@debug}` tag whose keyword ends at `keyword_end`. No
    /// whitespace is required after the keyword, and the tag may be
    /// empty.
    pub fn read_debug_tag(
        &self,
        keyword_end: u32,
        limit: u32,
    ) -> Result<ExpressionRead, TagReadError> {
        let at = self.skip_whitespace(keyword_end);
        if self.byte(at) == Some(b'}') {
            return Ok(ExpressionRead {
                expression_end: at,
                end: at + 1,
            });
        }
        self.read_expression_tag(keyword_end, limit)
    }

    /// A `{@const pattern = expression}` tag whose keyword ends at
    /// `keyword_end`.
    pub fn read_const_tag(
        &self,
        keyword_end: u32,
        limit: u32,
    ) -> Result<ExpressionRead, TagReadError> {
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

    /// A `{const …}` / `{let …}` declaration tag whose keyword starts at
    /// `start`, reading no further than `limit`.
    ///
    /// The compiler has acorn read a statement there. A variable
    /// declaration is the tag's content, and the `}` must follow it.
    /// Anything else acorn reads (`{let}` is the expression `let`) makes
    /// the tag an expression tag: `Ok(None)`.
    pub fn read_declaration_tag(
        &self,
        start: u32,
        limit: u32,
    ) -> Result<Option<ExpressionRead>, TagReadError> {
        let text = self
            .source
            .get(start as usize..limit as usize)
            .unwrap_or("");
        let read = AcornRead {
            range: Range::new(start, limit),
            shape: AcornReadShape::Statement,
        };
        let alloc = Allocator::default();
        let source_type = SourceType::default()
            .with_module(true)
            .with_typescript(self.ts);
        let parsed = oxc_parser::Parser::new(&alloc, text, source_type).parse();
        let whole = parsed.diagnostics.is_empty() && parsed.program.body.len() == 1;
        match parsed.program.body.first() {
            Some(oxc_ast::ast::Statement::VariableDeclaration(declaration)) if whole => {
                // acorn refuses declarations this reading lets through:
                // a name declared twice, a reserved word bound in strict
                // code, an invalid regular expression.
                let language = if self.ts {
                    svn_acorn::Language::TypeScript
                } else {
                    svn_acorn::Language::JavaScript
                };
                if let Err(error) = svn_acorn::parse_statement_at(text, 0, language) {
                    return Err(TagReadError::new(
                        start + error.pos,
                        TagReadErrorKind::JsParse(error.message),
                    )
                    .reading(read));
                }
                let expression_end = start + declaration.span.end;
                Ok(Some(ExpressionRead {
                    expression_end,
                    end: self.closing_brace(expression_end)?,
                }))
            }
            Some(_) if whole => Ok(None),
            first => {
                // acorn stops inside the statement, or after it where
                // the `}` should be.
                let at = parsed
                    .diagnostics
                    .first()
                    .and_then(|d| d.labels.first())
                    .map(|l| l.span().start)
                    .or_else(|| first.map(|statement| statement.span().end))
                    .unwrap_or(text.len() as u32);
                Err(TagReadError::new(
                    start + at,
                    TagReadErrorKind::JsParse("Unexpected token".to_string()),
                )
                .reading(read))
            }
        }
    }

    /// The attributes of an opening tag whose name ends at `name_end`, and
    /// the `>` or `/>` after them, as the compiler's `read_attribute`
    /// loop reads them: just past the `>`.
    ///
    /// Only the reading is replayed, for the errors it throws. Which
    /// attributes there are is still decided by the parser's own scan.
    pub fn read_open_tag(&self, name_end: u32) -> Result<u32, TagReadError> {
        let mut at = self.skip_whitespace(name_end);
        loop {
            // Comments between attributes belong to the next one, and
            // are read even when none follows.
            at = self.skip_attribute_comments(at);
            match self.read_attribute(at)? {
                Some(end) => at = self.skip_whitespace(end),
                None => break,
            }
        }
        if self.byte(at) == Some(b'/') {
            at += 1;
        }
        if self.byte(at) != Some(b'>') {
            return Err(TagReadError::new(at, TagReadErrorKind::ExpectedToken(">")));
        }
        Ok(at + 1)
    }

    /// Where the template ends for the compiler, which trims it.
    fn template_end(&self) -> u32 {
        self.source.trim_end().len() as u32
    }

    /// `read_tag_name()`: the end of the name at `at`. An attribute's
    /// name also stops at a quote or `=`.
    fn tag_name_end(&self, at: u32, attribute: bool) -> Result<u32, TagReadError> {
        let end = self.template_end();
        if at >= end {
            return Err(TagReadError::new(end, TagReadErrorKind::UnexpectedEof));
        }
        let rest = &self.source[at as usize..end as usize];
        let len = rest
            .char_indices()
            .find(|&(_, c)| {
                is_compiler_whitespace(c)
                    || matches!(c, '/' | '>')
                    || (attribute && matches!(c, '"' | '\'' | '='))
            })
            .map_or(rest.len(), |(i, _)| i);
        Ok(at + len as u32)
    }

    /// The expression of an attribute's `{…}` from `start`, up to its
    /// `}`: just past it.
    fn attribute_expression(&self, start: u32) -> Result<u32, TagReadError> {
        // acorn stops at an unpaired `}`, so no read goes past the one a
        // brace count finds.
        let limit = crate::mustache::find_mustache_end(self.source, start)
            .unwrap_or_else(|| self.template_end());
        Ok(self.read_expression_tag(start, limit)?.end)
    }

    /// The `//` and `/* */` comments the compiler reads before an
    /// attribute, with the whitespace after each: where they end.
    fn skip_attribute_comments(&self, mut at: u32) -> u32 {
        loop {
            let rest = self.source.get(at as usize..).unwrap_or("");
            if rest.starts_with("//") {
                at += rest.find('\n').unwrap_or(rest.len()) as u32;
            } else if rest.starts_with("/*") {
                at += rest.find("*/").map_or(rest.len(), |i| i + 2) as u32;
            } else {
                break;
            }
            at = self.skip_whitespace(at);
        }
        at
    }

    /// `read_attribute()` at `at`, past its comments: where the
    /// attribute ends, or `None` when no attribute starts there.
    fn read_attribute(&self, at: u32) -> Result<Option<u32>, TagReadError> {
        let start = at;
        if self.byte(at) == Some(b'{') {
            let inner = self.skip_whitespace(at + 1);
            let rest = &self.source[inner as usize..];
            if rest.starts_with("@attach") {
                let expression = self.require_whitespace(inner + 7)?;
                return self.attribute_expression(expression).map(Some);
            }
            if rest.starts_with("...") {
                return self.attribute_expression(inner + 3).map(Some);
            }
            let name_end = self.read_identifier(inner)?;
            if name_end == inner {
                return Err(TagReadError::new(
                    start,
                    TagReadErrorKind::AttributeEmptyShorthand,
                ));
            }
            return self.closing_brace(name_end).map(Some);
        }
        let name_end = self.tag_name_end(at, true)?;
        if name_end == at {
            return Ok(None);
        }
        let name = &self.source[at as usize..name_end as usize];
        let after = self.skip_whitespace(name_end);
        let mut end = name_end;
        // The value: where its first chunk starts, and whether it holds
        // text (anything but one `{expression}`).
        let mut value: Option<(u32, bool)> = None;
        if self.byte(after) == Some(b'=') {
            let value_start = self.skip_whitespace(after + 1);
            if self.source[value_start as usize..].starts_with("/>") {
                // `a=/>`: the value is the `/`.
                end = value_start + 1;
                value = Some((value_start, true));
            } else {
                let read = self.read_attribute_value(value_start)?;
                end = read.0;
                value = read.1;
            }
        } else if matches!(self.byte(after), Some(b'"' | b'\'')) {
            return Err(TagReadError::new(
                after,
                TagReadErrorKind::ExpectedToken("="),
            ));
        }
        if let Some(colon) = name.find(':') {
            let directive = matches!(
                &name[..colon],
                "use"
                    | "animate"
                    | "bind"
                    | "class"
                    | "style"
                    | "on"
                    | "let"
                    | "in"
                    | "out"
                    | "transition"
            );
            if directive {
                let directive_name = name[colon + 1..].split('|').next().unwrap_or_default();
                if directive_name.is_empty() {
                    return Err(TagReadError::new(
                        start,
                        TagReadErrorKind::DirectiveMissingName {
                            name: name.to_string(),
                            end: start + colon as u32 + 1,
                        },
                    ));
                }
                if &name[..colon] != "style"
                    && let Some((first, true)) = value
                {
                    return Err(TagReadError::new(
                        first,
                        TagReadErrorKind::DirectiveInvalidValue,
                    ));
                }
            }
        }
        Ok(Some(end))
    }

    /// `read_attribute_value()` at `at`: where the value ends, and where
    /// its first chunk starts with whether the value holds text. `None`
    /// for an empty quoted value, which is one empty text chunk.
    #[allow(clippy::type_complexity)]
    fn read_attribute_value(&self, at: u32) -> Result<(u32, Option<(u32, bool)>), TagReadError> {
        let quote = self.byte(at).filter(|b| matches!(b, b'"' | b'\''));
        let mut index = at + u32::from(quote.is_some());
        if let Some(quote) = quote
            && self.byte(index) == Some(quote)
        {
            return Ok((index + 1, Some((index, true))));
        }
        let end = self.template_end();
        let closing = match quote {
            Some(b'"') => "\"",
            Some(_) => "'",
            None => "}",
        };
        let mut chunk_start = index;
        // The chunks: (start, is text).
        let mut chunks: Vec<(u32, bool)> = Vec::new();
        let mut done = false;
        while index < end {
            let c = self.char_at(index).unwrap_or('\0');
            let stops = match quote {
                Some(quote) => c == char::from(quote),
                None => {
                    is_compiler_whitespace(c)
                        || matches!(c, '"' | '\'' | '=' | '<' | '>' | '`')
                        || self.source[index as usize..].starts_with("/>")
                }
            };
            if stops {
                done = true;
                break;
            }
            if c != '{' {
                index += c.len_utf8() as u32;
                continue;
            }
            let brace = index;
            let sigil = self.byte(brace + 1);
            if matches!(sigil, Some(b'#' | b'@')) {
                let name: String = self.source[brace as usize + 2..]
                    .chars()
                    .take_while(char::is_ascii_lowercase)
                    .collect();
                let location = "in attribute value";
                let kind = if sigil == Some(b'#') {
                    TagReadErrorKind::BlockInvalidPlacement { name, location }
                } else {
                    TagReadErrorKind::TagInvalidPlacement { name, location }
                };
                return Err(TagReadError::new(brace, kind));
            }
            if brace > chunk_start {
                chunks.push((chunk_start, true));
            }
            index = match self.attribute_expression(self.skip_whitespace(brace + 1)) {
                Ok(end) => end,
                Err(error) => {
                    // An expression that runs into the tag's `/>`: acorn
                    // fails on the `/`, and the compiler says what was
                    // missing before it.
                    let slash = error.at.saturating_sub(1);
                    if matches!(error.kind, TagReadErrorKind::JsParse(_))
                        && self.source.get(slash as usize..slash as usize + 2) == Some("/>")
                    {
                        return Err(TagReadError::new(
                            error.at,
                            TagReadErrorKind::ExpectedToken(closing),
                        ));
                    }
                    return Err(error);
                }
            };
            chunks.push((brace, false));
            chunk_start = index;
        }
        if !done {
            return Err(TagReadError::new(end, TagReadErrorKind::UnexpectedEof));
        }
        if index > chunk_start {
            chunks.push((chunk_start, true));
        }
        if chunks.is_empty() && quote.is_none() {
            return Err(TagReadError::new(
                index,
                TagReadErrorKind::ExpectedAttributeValue,
            ));
        }
        let holds_text = chunks.len() > 1 || chunks.first().is_some_and(|c| c.1);
        let first = chunks.first().map(|c| (c.0, holds_text));
        Ok((index + u32::from(quote.is_some()), first))
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
