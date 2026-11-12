//! The compiler's errors for the reads inside a tag.
//!
//! The reads themselves are [`svn_parser::tag_read`]'s. The parser ends
//! and splits every tag with them and keeps the compiler's error on the
//! node; `From<TagReadError>` turns it into a lint finding.

use svn_parser::tag_read::{TagReadError, TagReadErrorKind};

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
