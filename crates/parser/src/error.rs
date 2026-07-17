//! Parser errors.
//!
//! Kept as concrete variants with ranges so diagnostics can be reported with
//! accurate source spans. All errors implement `std::error::Error` via
//! `thiserror`.
//!
//! ## Coverage vs the Svelte compiler's parse errors
//!
//! The compiler defines on the order of a hundred parse-time errors
//! (`compiler/errors.js`); this enum ports only the handful whose
//! absence would silently corrupt a parse (unterminated constructs,
//! duplicate sections, mismatched/stray tags). Malformed-input
//! diagnostic parity is deliberately best-effort: the mandate is parity
//! on code the compiler accepts, and a file that fails to compile
//! upstream gets its authoritative errors from the compiler itself.
//! The unported families, so the gap is explicit rather than
//! accidental:
//!
//! - Token/expression detail errors: `expected_token`,
//!   `expected_identifier`, `expected_pattern`, `expected_whitespace`,
//!   `expected_block_type`, `expected_tag`, `expected_attribute_value`,
//!   `js_parse_error`, `unexpected_reserved_word`,
//!   `unterminated_string_constant`, `block_unexpected_character`.
//! - Block-shape validation: `block_invalid_elseif`,
//!   `block_duplicate_clause`, `block_unclosed` (we report
//!   `UnterminatedElement` instead), `each_key_without_as`,
//!   `illegal_await_expression`.
//! - Placement validation: `block_invalid_placement`,
//!   `tag_invalid_placement`, `node_invalid_placement`,
//!   `const_tag_invalid_placement`, `let_directive_invalid_placement`,
//!   `title_invalid_content`, `textarea_invalid_content` (we report
//!   `MalformedOpenTag` there), `void_element_invalid_content`,
//!   `svelte_meta_invalid_placement`, `svelte_self_invalid_placement`,
//!   `svelte_fragment_invalid_placement`.
//! - Attribute/directive validation: `attribute_duplicate`,
//!   `attribute_invalid_name`, `attribute_empty_shorthand`,
//!   `attribute_invalid_sequence_expression`,
//!   `attribute_unquoted_sequence`, `directive_invalid_value`,
//!   `directive_missing_name`, the `bind_*`, `event_handler_*`,
//!   `animation_*`, `transition_*` and `style_directive_*` families.
//! - `svelte:*` / slot validation: the `svelte_component_*`,
//!   `svelte_element_*`, `svelte_options_*`, `svelte_boundary_*`,
//!   `svelte_meta_*`, `svelte_head`/`svelte_body`, `slot_*` and
//!   `title_illegal_attribute` families.
//! - Script/style tag validation: `script_invalid_attribute_value`,
//!   `script_reserved_attribute`, `script_invalid_context` (we report
//!   `UnknownScriptContext`).
//! - Snippet/tag declarations: `snippet_*`, `render_tag_*`,
//!   `declaration_tag_*`, `const_tag_*` (non-placement),
//!   `debug_tag_invalid_arguments`, `legacy_await_invalid`,
//!   `experimental_async`.
//!
//! Analyze-phase errors (rune misuse, store/prop rules, CSS) are
//! `svn-lint`'s concern, not the parser's.

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

    #[error("Unexpected block closing tag")]
    UnexpectedBlockClose { range: Range },

    #[error(
        "{{:...}} block is invalid at this position (did you forget to close the preceding element or block?)"
    )]
    InvalidBlockContinuation { range: Range },

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
            Self::UnexpectedBlockClose { range } => *range,
            Self::InvalidBlockContinuation { range } => *range,
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
            Self::UnexpectedBlockClose { .. } => "block-unexpected-close",
            Self::InvalidBlockContinuation { .. } => "block-invalid-continuation-placement",
            Self::UnknownSvelteElement { .. } => "unknown-svelte-element",
            Self::UnsupportedBlock { .. } => "unsupported-block",
        }
    }
}
