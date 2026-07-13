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

    #[error("Unexpected end of input")]
    UnexpectedEof { range: Range },

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
            Self::UnexpectedEof { range } => *range,
            Self::UnterminatedElement { range, .. } => *range,
            Self::MismatchedClosingTag { range, .. } => *range,
            Self::UnknownSvelteElement { range, .. } => *range,
            Self::UnsupportedBlock { range } => *range,
        }
    }

    /// Whether this is a genuine user syntax error (upstream's compiler
    /// would throw) versus an internal-limitation marker we must not
    /// surface as a user diagnostic.
    ///
    /// `UnsupportedBlock` flags a construct this build can't parse yet —
    /// an internal gap, not malformed user input. Everything else is a
    /// real syntax error (unterminated tag/mustache/comment, duplicate
    /// script/style, mismatched close, malformed open, unknown
    /// `svelte:*` element, bad script context/lang).
    pub fn is_fatal(&self) -> bool {
        !matches!(self, Self::UnsupportedBlock { .. })
    }

    /// A stable kebab-case slug per variant, used as the diagnostic
    /// `code`. Best-effort identifiers for our native reimplementation;
    /// `bridge` mode emits upstream's exact compiler codes instead.
    pub fn code_slug(&self) -> &'static str {
        match self {
            Self::UnterminatedTag { .. } => "unterminated-tag",
            Self::DuplicateScript { .. } => "duplicate-script",
            Self::DuplicateStyle { .. } => "duplicate-style",
            Self::MalformedOpenTag { .. } => "malformed-open-tag",
            Self::UnknownScriptContext { .. } => "unknown-script-context",
            Self::UnknownScriptLang { .. } => "unknown-script-lang",
            Self::UnterminatedComment { .. } => "unterminated-comment",
            Self::UnterminatedMustache { .. } => "unterminated-mustache",
            Self::UnexpectedEof { .. } => "unexpected-eof",
            Self::UnterminatedElement { .. } => "unterminated-element",
            Self::MismatchedClosingTag { .. } => "mismatched-closing-tag",
            Self::UnknownSvelteElement { .. } => "unknown-svelte-element",
            Self::UnsupportedBlock { .. } => "unsupported-block",
        }
    }
}
