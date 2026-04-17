//! Top-level document types.
//!
//! A Svelte file decomposes into at most three "opaque" sections —
//! `<script context="module">`, `<script>`, `<style>` — and a template
//! region that's everything else. This module defines those shapes; the
//! structural parser in `sections.rs` populates them.

use svn_core::Range;

/// A parsed Svelte file.
///
/// Borrows from the source string (`&'src`). Template AST is out of scope
/// for the initial structural parser; it will live in [`Template::nodes`]
/// once the template-level parser lands.
#[derive(Debug)]
pub struct Document<'src> {
    /// The original source text. Every `Range` in the document is an offset
    /// into this string.
    pub source: &'src str,

    /// `<script context="module">` if present.
    pub module_script: Option<ScriptSection<'src>>,

    /// `<script>` (instance) if present.
    pub instance_script: Option<ScriptSection<'src>>,

    /// `<style>` if present.
    pub style: Option<StyleSection<'src>>,

    /// Template contents — everything outside the opaque sections.
    pub template: Template,
}

/// A `<script>` block.
#[derive(Debug, Clone)]
pub struct ScriptSection<'src> {
    /// Range spanning the opening tag, including `<` and `>`.
    pub open_tag_range: Range,
    /// Range of the script *body* (between `>` and `</script>`).
    pub content_range: Range,
    /// Range spanning the closing `</script>` tag.
    pub close_tag_range: Range,

    /// The body text. Equal to `content_range.slice(source)` — cached for
    /// convenience and to give downstream crates (oxc) a plain `&str`.
    pub content: &'src str,

    /// Parsed `lang=` attribute.
    pub lang: ScriptLang,
    /// Parsed `context=` attribute.
    pub context: ScriptContext,

    /// Parsed `generics="..."` attribute (Svelte 5 only). Holds the raw
    /// type-parameter-list string verbatim — e.g. `"T, K extends keyof T"`
    /// — trimmed of surrounding whitespace. `None` if the attribute is
    /// absent or empty. The value is spliced directly into the wrapping
    /// render function as `function $$render<T, K extends keyof T>() { ... }`.
    ///
    /// Per Svelte 5, the attribute is only meaningful on the INSTANCE
    /// script; setting it on a `<script module>` is a user error (we
    /// don't emit a diagnostic for it yet, but the field is only
    /// populated on instance scripts).
    pub generics: Option<String>,

    /// Every attribute found on the opening tag, including ones we don't
    /// interpret. Preserved so diagnostics can echo them back and the
    /// emitter has full fidelity if ever needed.
    pub attrs: Vec<ScriptAttr>,
}

/// A `<style>` block. For now we only record its range; css parsing lives in
/// the `lint` crate.
#[derive(Debug, Clone)]
pub struct StyleSection<'src> {
    pub open_tag_range: Range,
    pub content_range: Range,
    pub close_tag_range: Range,
    pub content: &'src str,
    pub attrs: Vec<ScriptAttr>,
}

/// An attribute as written on an opaque-section tag. Shape-only: no
/// interpretation of value contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptAttr {
    pub name: String,
    /// `None` → valueless attribute (e.g. `<script defer>`).
    /// `Some("")` → explicit empty (`<script lang="">`).
    pub value: Option<String>,
    pub range: Range,
}

/// Script language. `Js` is the default for a `<script>` tag with no `lang=`
/// or `lang="js"`/`lang="javascript"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptLang {
    Js,
    Ts,
}

impl ScriptLang {
    /// Return the oxc source-type string (for wiring into oxc_parser later).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Js => "js",
            Self::Ts => "ts",
        }
    }
}

/// Script context. `Instance` is the default for a bare `<script>` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptContext {
    Instance,
    Module,
}

/// The template region. `nodes` will be populated by a subsequent pass
/// (template AST not included in the initial structural parser).
#[derive(Debug, Default)]
pub struct Template {
    /// Byte ranges in the source that belong to the template — the
    /// complement of script/style sections. Stored as a list because
    /// template content can be interleaved with script/style blocks.
    pub text_runs: Vec<Range>,
}
