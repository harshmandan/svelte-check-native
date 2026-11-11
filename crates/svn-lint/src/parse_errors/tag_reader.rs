//! The compiler's errors for the reads inside a block tag.
//!
//! The reads themselves are [`svn_parser::tag_read`]'s. The parser ends
//! and splits the `{#each}`, `{#await}`, `{:then}`, `{:catch}` and
//! `{#snippet}` tags with them and keeps the compiler's error on the node
//! (see `From<TagReadError>`); the keyword whitespace of the other tags
//! and the pattern of a `{@const}` are read here, from where our parser
//! placed them. Each function returns the first error the compiler throws
//! there, as a lint finding.

use svn_parser::ast::Interpolation;
use svn_parser::tag_read::{self, Comments, TagReadError, TagReadErrorKind};

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
}

impl From<TagReadError> for TagError {
    fn from(error: TagReadError) -> Self {
        let at = error.at;
        let (code, message) = match error.kind {
            TagReadErrorKind::ExpectedToken(token) => {
                (Code::expected_token, messages::expected_token(token))
            }
            TagReadErrorKind::ExpectedWhitespace => {
                (Code::expected_whitespace, messages::expected_whitespace())
            }
            TagReadErrorKind::ExpectedPattern => {
                (Code::expected_pattern, messages::expected_pattern())
            }
            TagReadErrorKind::ExpectedIdentifier => {
                (Code::expected_identifier, messages::expected_identifier())
            }
            TagReadErrorKind::ReservedWord(word) => (
                Code::unexpected_reserved_word,
                messages::unexpected_reserved_word(&word),
            ),
            TagReadErrorKind::UnexpectedEof => (Code::unexpected_eof, messages::unexpected_eof()),
            TagReadErrorKind::UnterminatedStringConstant => (
                Code::unterminated_string_constant,
                messages::unterminated_string_constant(),
            ),
            TagReadErrorKind::JsParse(message) => (
                Code::js_parse_error,
                format!("{message}\nhttps://svelte.dev/e/js_parse_error"),
            ),
        };
        Self { at, code, message }
    }
}

pub(super) struct TagReader<'s> {
    source: &'s str,
    reader: tag_read::TagReader<'s>,
}

impl<'s> TagReader<'s> {
    /// `ts`: the compiler reads the component's JavaScript as TypeScript.
    pub(super) fn new(source: &'s str, ts: bool) -> Self {
        Self {
            source,
            reader: tag_read::TagReader::new(source, ts),
        }
    }

    /// The end of the tag keyword `keyword` written after the tag's
    /// `{` and `sigil` (`#`, `@`), when the tag starts that way.
    fn keyword_end(&self, tag_start: u32, sigil: u8, keyword: &str) -> Option<u32> {
        let at = self.reader.skip_whitespace(tag_start + 1);
        if self.reader.byte(at) != Some(sigil) {
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
        self.reader
            .require_whitespace(end)
            .err()
            .map(TagError::from)
    }

    /// The whitespace that must precede what our parser read as the
    /// part of a tag after its keyword (`{:else if cond}`, `{:then v}`),
    /// which starts right after that whitespace.
    pub(super) fn whitespace_before_part(&self, part_start: u32) -> Option<TagError> {
        (!self.reader.whitespace_before(part_start))
            .then(|| TagError::expected_whitespace(part_start))
    }

    /// `{@const pattern = init}`, up to its `=`.
    pub(super) fn const_tag(&self, tag: &Interpolation) -> Option<TagError> {
        let keyword_end = self.keyword_end(tag.range.start, b'@', "const")?;
        let start = match self.reader.require_whitespace(keyword_end) {
            Ok(start) => start,
            Err(e) => return Some(e.into()),
        };
        let close = tag.range.end.checked_sub(1)?;
        let end = match self
            .reader
            .read_pattern(start, close, &mut Comments::default())
        {
            Ok((_, end)) => end,
            Err(e) => return Some(e.into()),
        };
        let at = self.reader.skip_whitespace(end);
        (self.reader.byte(at) != Some(b'=')).then(|| TagError::expected_token(at, "="))
    }
}
