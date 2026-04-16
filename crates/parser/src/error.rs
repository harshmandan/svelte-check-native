//! Parser errors.
//!
//! Kept as concrete variants with ranges so diagnostics can be reported with
//! accurate source spans. All errors implement `std::error::Error` via
//! `thiserror`.

use svn_core::Range;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("unterminated {tag_name} tag (no matching </{tag_name}>)")]
    UnterminatedTag {
        tag_name: &'static str,
        range: Range,
    },

    #[error("duplicate <script{descriptor}> block")]
    DuplicateScript {
        descriptor: &'static str,
        range: Range,
    },

    #[error("duplicate <style> block")]
    DuplicateStyle { range: Range },

    #[error("malformed opening tag")]
    MalformedOpenTag { range: Range },

    #[error("unknown script context {value:?}; expected \"module\" or nothing")]
    UnknownScriptContext { value: String, range: Range },

    #[error("unknown script lang {value:?}; expected \"ts\", \"typescript\", \"js\", or nothing")]
    UnknownScriptLang { value: String, range: Range },

    #[error("unterminated HTML comment")]
    UnterminatedComment { range: Range },

    #[error("unterminated mustache expression (no matching `}}`)")]
    UnterminatedMustache { range: Range },

    #[error("unterminated <{name}> element (no matching </{name}>)")]
    UnterminatedElement { name: String, range: Range },

    #[error("mismatched closing tag, expected </{expected}>")]
    MismatchedClosingTag { expected: String, range: Range },

    #[error("unknown <svelte:{name}> element")]
    UnknownSvelteElement { name: String, range: Range },

    #[error("block-level mustache ({{#}}/{{:}}/{{/}}/{{@}}) not yet supported in this build")]
    UnsupportedBlock { range: Range },
}

impl ParseError {
    /// The source range this error points at.
    pub fn range(&self) -> Range {
        match self {
            Self::UnterminatedTag { range, .. } => *range,
            Self::DuplicateScript { range, .. } => *range,
            Self::DuplicateStyle { range, .. } => *range,
            Self::MalformedOpenTag { range } => *range,
            Self::UnknownScriptContext { range, .. } => *range,
            Self::UnknownScriptLang { range, .. } => *range,
            Self::UnterminatedComment { range } => *range,
            Self::UnterminatedMustache { range } => *range,
            Self::UnterminatedElement { range, .. } => *range,
            Self::MismatchedClosingTag { range, .. } => *range,
            Self::UnknownSvelteElement { range, .. } => *range,
            Self::UnsupportedBlock { range } => *range,
        }
    }
}
