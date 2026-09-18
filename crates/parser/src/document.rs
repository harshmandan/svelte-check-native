//! Top-level document types.
//!
//! A Svelte file decomposes into at most three "opaque" sections —
//! `<script context="module">`, `<script>`, `<style>` — and a template
//! region that's everything else. This module defines those shapes; the
//! structural parser in `sections.rs` populates them.

use svn_core::Range;

/// A parsed Svelte file.
///
/// Borrows from the source string (`&'src`). The structural parser records
/// the template only as byte ranges (`Template::text_runs`); the template AST
/// is produced separately by `parse_all_template_runs`, which turns those runs
/// into an `ast::Fragment`.
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

impl<'src> Document<'src> {
    /// Effective script language for emit/cache decisions.
    ///
    /// Picks the instance script's `lang=` when present (the dominant
    /// case — instance script holds runtime logic). Falls back to the
    /// module script when there's no instance. With neither script tag
    /// present the file is treated as JS, matching upstream's
    /// "no script = JS" behaviour (the resulting overlay carries no
    /// user code anyway, only template scaffolding).
    pub fn script_lang(&self) -> ScriptLang {
        if let Some(s) = self.instance_script.as_ref() {
            return s.lang;
        }
        if let Some(s) = self.module_script.as_ref() {
            return s.lang;
        }
        ScriptLang::Js
    }
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

/// The template region. The structural parser stores only `text_runs` — the
/// byte ranges that belong to the template. The template AST is built later by
/// `parse_all_template_runs`, which parses those runs into an `ast::Fragment`.
#[derive(Debug, Default)]
pub struct Template {
    /// Byte ranges in the source that belong to the template — the
    /// complement of script/style sections. Stored as a list because
    /// template content can be interleaved with script/style blocks.
    pub text_runs: Vec<Range>,
}

/// svelte-check's `isTsSvelte`: the component is TypeScript when ANY
/// `<script …>` tag in its text — wherever it sits, comments included —
/// carries `lang` `ts` or `typescript` (any case). A literal port of its
/// two regular expressions:
///
/// ```text
/// /<script\b((?:\s+[^=>'"\/\s]+(?:=(?:"[^"]*"|'[^']*'|[^>\s]+))?)*)\s*>/gi
/// /\blang\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))/i
/// ```
pub fn is_ts_svelte(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(rel) = find_ascii_ci(&bytes[from..], b"<script") {
        let start = from + rel;
        from = start + 1;
        if let Some(attrs) = script_tag_attrs(text, start + b"<script".len())
            && let Some(lang) = lang_value(attrs)
            && (lang.eq_ignore_ascii_case("ts") || lang.eq_ignore_ascii_case("typescript"))
        {
            return true;
        }
    }
    false
}

fn find_ascii_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The attribute text of a `<script` tag whose name ends at `at`, when
/// the rest of the tag matches the first expression. The expression
/// admits exactly one way to match, so a greedy walk decides it.
fn script_tag_attrs(text: &str, at: usize) -> Option<&str> {
    let rest = &text[at..];
    if rest.chars().next().is_some_and(is_word_char) {
        return None;
    }
    let mut pos = 0;
    loop {
        let ws = rest[pos..].len() - rest[pos..].trim_start().len();
        let name_start = pos + ws;
        let name_len = rest[name_start..]
            .find(|c: char| matches!(c, '=' | '>' | '\'' | '"' | '/') || c.is_whitespace())
            .unwrap_or(rest.len() - name_start);
        if ws == 0 || name_len == 0 {
            break;
        }
        let mut end = name_start + name_len;
        if rest[end..].starts_with('=') {
            let value = &rest[end + 1..];
            let len = match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].find(q).map(|i| i + 2),
                Some(_) => {
                    let n = value
                        .find(|c: char| c == '>' || c.is_whitespace())
                        .unwrap_or(value.len());
                    (n > 0).then_some(n)
                }
                None => None,
            };
            match len {
                Some(len) => end += 1 + len,
                // `=` with no value: the optional group can't match, and
                // `=` can't start the closing `\s*>` either.
                None => return None,
            }
        }
        pos = end;
    }
    let closing = rest[pos..].trim_start();
    closing.starts_with('>').then(|| &rest[..pos])
}

/// The first `lang=value` match in a tag's attribute text.
fn lang_value(attrs: &str) -> Option<&str> {
    let lower = attrs.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("lang") {
        let at = from + rel;
        from = at + 1;
        if attrs[..at].chars().next_back().is_some_and(is_word_char) {
            continue;
        }
        let after = attrs[at + 4..].trim_start();
        let Some(after) = after.strip_prefix('=') else {
            continue;
        };
        let value = after.trim_start();
        let found = match value.chars().next() {
            Some(q @ ('"' | '\'')) => value[1..].find(q).map(|i| &value[1..=i]),
            Some(_) => {
                let n = value
                    .find(|c: char| {
                        c.is_whitespace() || matches!(c, '"' | '\'' | '=' | '<' | '>' | '`')
                    })
                    .unwrap_or(value.len());
                (n > 0).then(|| &value[..n])
            }
            None => None,
        };
        if found.is_some() {
            return found;
        }
    }
    None
}

#[cfg(test)]
mod is_ts_svelte_tests {
    use super::is_ts_svelte;

    #[test]
    fn follows_svelte_checks_expressions() {
        assert!(is_ts_svelte(r#"<script lang="ts">x</script>"#));
        assert!(is_ts_svelte(r#"<SCRIPT LANG='TypeScript'>"#));
        assert!(is_ts_svelte("<script context=module lang=ts >"));
        assert!(is_ts_svelte(
            r#"<script module lang="ts"></script><script>let a</script>"#
        ));
        assert!(is_ts_svelte(
            r#"<!-- <script lang="ts"> --><script></script>"#
        ));
        assert!(is_ts_svelte(r#"<script data-x="lang=ts">"#));
        assert!(!is_ts_svelte("<script>let lang = 'ts'</script>"));
        assert!(!is_ts_svelte(r#"<scripts lang="ts">"#));
        assert!(!is_ts_svelte(r#"<script lang="ts" />"#));
        assert!(!is_ts_svelte(r#"<script lang="js">"#));
        assert!(!is_ts_svelte(r#"<script xlang="ts">"#));
        assert!(is_ts_svelte(r#"<script x-lang="ts">"#));
    }
}
