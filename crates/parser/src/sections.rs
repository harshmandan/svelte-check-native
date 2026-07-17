//! Top-level `<script>` / `<style>` section machinery.
//!
//! Section-ness is decided inside the template parser: [`parse_sections`]
//! drives one document-mode walk of the whole source
//! ([`crate::template::scan_document_sections`]), and a `<script>` /
//! `<style>` open tag encountered while the parser's open-frame stack is
//! empty — no element open, no block open — is a top-level section. That
//! is the same rule the Svelte compiler applies (`element.js`: sections
//! iff `current.type === 'Root'`, where elements and blocks both push a
//! parser-stack frame). Everything else — an analytics snippet under
//! `<svelte:head>`, a CDN loader gated behind `{#if}`, a commented-out
//! reference implementation — is a template element (or comment text) by
//! construction, because the template parser already knows every
//! construct that can enclose a tag.
//!
//! This module owns what happens once a tag IS a section: the opaque
//! read of `<TAG ...>body</TAG>` (HTML rules — everything up to the
//! matching case-insensitive close tag is uninterpreted, matching
//! browser parsing and Svelte's own), attribute / lang / context /
//! generics interpretation, duplicate-section errors, and accumulation
//! of the template text runs (everything not claimed by a section).
//! Script bodies are recovered verbatim so oxc can parse them; style
//! bodies are recovered for CSS validation.

use svn_core::Range;

use crate::document::{
    Document, ScriptAttr, ScriptContext, ScriptLang, ScriptSection, StyleSection, Template,
};
use crate::error::ParseError;
use crate::scanner::Scanner;

/// Parse the top-level section layout of a Svelte source file.
///
/// Returns a [`Document`] plus any structural errors encountered. The
/// document is always returned (even if errors exist) so downstream crates
/// can still inspect partial state — e.g., an instance script that was
/// otherwise OK even though the style block was malformed.
///
/// Only section-level errors (duplicates, unterminated tags, unknown
/// lang/context) are reported here; template-structure errors surface
/// when `parse_all_template_runs` parses the returned text runs.
pub fn parse_sections(source: &str) -> (Document<'_>, Vec<ParseError>) {
    let mut collector = SectionCollector::new(source);
    crate::template::scan_document_sections(source, &mut collector);
    collector.into_document()
}

/// If the scanner sits on a `<script` / `<style` tag start, the section
/// tag name — else `None`. The identifier-boundary check distinguishes
/// `<script>` from e.g. `<scripted>`. Whether the position actually IS a
/// section is the template parser's call (root frame only); this only
/// answers "is this one of the two section tag names?".
pub(crate) fn section_tag_at(scanner: &Scanner<'_>) -> Option<&'static str> {
    if scanner.peek_byte() != Some(b'<') {
        return None;
    }
    if scanner.starts_with_ignore_case("<script") && !is_ident_char(scanner.peek_byte_at(7)) {
        return Some("script");
    }
    if scanner.starts_with_ignore_case("<style") && !is_ident_char(scanner.peek_byte_at(6)) {
        return Some("style");
    }
    None
}

/// Accumulates sections and template text runs during the document-mode
/// template walk. The template parser calls [`SectionCollector::claim`]
/// whenever a `<script>`/`<style>` open tag surfaces at the document
/// root; every byte not claimed by a section ends up in `text_runs`.
pub(crate) struct SectionCollector<'src> {
    source: &'src str,
    module_script: Option<ScriptSection<'src>>,
    instance_script: Option<ScriptSection<'src>>,
    style: Option<StyleSection<'src>>,
    text_runs: Vec<Range>,
    /// Start of the pending (not yet flushed) template text run.
    template_cursor: u32,
    errors: Vec<ParseError>,
}

impl<'src> SectionCollector<'src> {
    pub(crate) fn new(source: &'src str) -> Self {
        Self {
            source,
            module_script: None,
            instance_script: None,
            style: None,
            text_runs: Vec::new(),
            template_cursor: 0,
            errors: Vec::new(),
        }
    }

    /// Claim the section whose `<` the scanner sits on. `tag` is
    /// `"script"` or `"style"` as reported by [`section_tag_at`]. On
    /// success the scanner ends up just past `</TAG>`; on error the
    /// unclaimed bytes stay template text (the cursor is not advanced)
    /// and the scanner moves at least one byte so the walk progresses.
    pub(crate) fn claim(&mut self, scanner: &mut Scanner<'src>, tag: &'static str) {
        // Flush pending template text.
        let here = scanner.pos();
        if self.template_cursor < here {
            self.text_runs.push(Range::new(self.template_cursor, here));
        }
        match parse_opaque_section(scanner, tag) {
            Ok(raw) if tag == "script" => {
                let section = build_script_section(self.source, raw, &mut self.errors);
                let is_module = section.context == ScriptContext::Module;
                let open_range = section.open_tag_range;
                let close_range = section.close_tag_range;
                let is_duplicate_script = if is_module {
                    let dup = self.module_script.is_some();
                    if dup {
                        self.errors.push(ParseError::DuplicateScript {
                            descriptor: " context=\"module\"",
                            range: open_range,
                        });
                    } else {
                        self.module_script = Some(section);
                    }
                    dup
                } else if self.instance_script.is_some() {
                    self.errors.push(ParseError::DuplicateScript {
                        descriptor: "",
                        range: open_range,
                    });
                    true
                } else {
                    self.instance_script = Some(section);
                    false
                };
                // A duplicate here is a genuine second document-level
                // script (nested ones never reach the collector — the
                // template parser routes them to the element path).
                // Recovery: surface the extra tag's spans as template
                // runs so its attribute expressions still flow through
                // the template-ref pass and any bindings they reference
                // aren't flagged as TS6133 "declared but never read".
                if is_duplicate_script {
                    self.text_runs
                        .push(Range::new(open_range.start, open_range.end));
                    if close_range.start < close_range.end {
                        self.text_runs
                            .push(Range::new(close_range.start, close_range.end));
                    }
                }
                self.template_cursor = scanner.pos();
            }
            Ok(raw) => {
                let section = build_style_section(self.source, raw);
                if self.style.is_some() {
                    self.errors.push(ParseError::DuplicateStyle {
                        range: section.open_tag_range,
                    });
                } else {
                    self.style = Some(section);
                }
                self.template_cursor = scanner.pos();
            }
            Err(err) => {
                self.errors.push(err);
                // Step past wherever the opaque read stopped so the
                // document walk can't loop; the bytes re-enter the walk
                // as template text.
                scanner.advance_byte();
            }
        }
    }

    /// Flush the trailing template text run. `end` is the document
    /// walk's final scanner position.
    pub(crate) fn finish(&mut self, end: u32) {
        if self.template_cursor < end {
            self.text_runs.push(Range::new(self.template_cursor, end));
        }
    }

    pub(crate) fn into_document(self) -> (Document<'src>, Vec<ParseError>) {
        (
            Document {
                source: self.source,
                module_script: self.module_script,
                instance_script: self.instance_script,
                style: self.style,
                template: Template {
                    text_runs: self.text_runs,
                },
            },
            self.errors,
        )
    }
}

/// Is the byte a character that could be part of an HTML tag-name identifier?
/// Used to distinguish `<script>` from e.g. `<scriptable>`.
fn is_ident_char(byte: Option<u8>) -> bool {
    match byte {
        Some(b) => b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'),
        None => false,
    }
}

/// Raw, uninterpreted section data — shape-only. Attribute values and bodies
/// are verbatim slices of the source. Interpretation into typed fields
/// happens in `build_*_section`.
struct RawSection<'src> {
    open_tag_range: Range,
    content_range: Range,
    close_tag_range: Range,
    content: &'src str,
    attrs: Vec<ScriptAttr>,
}

/// Parse `<TAG ...>...body...</TAG>` where TAG is `script` or `style`. The
/// scanner must be positioned at the opening `<`. On return the scanner
/// points just past `</TAG>`.
fn parse_opaque_section<'src>(
    scanner: &mut Scanner<'src>,
    tag_name: &'static str,
) -> Result<RawSection<'src>, ParseError> {
    let open_start = scanner.pos();
    let (lead, close_tag_literal): (&'static str, &'static str) = match tag_name {
        "script" => ("<script", "</script"),
        "style" => ("<style", "</style"),
        // parse_opaque_section is only ever called with "script"/"style".
        _ => unreachable!("parse_opaque_section called with non-opaque tag"),
    };
    // Eat `<tagname` (case-insensitive).
    debug_assert!(
        scanner.starts_with_ignore_case(lead),
        "parse_opaque_section called with wrong tag"
    );
    scanner.advance(lead.len() as u32);

    let (attrs, self_closing, open_end) = parse_tag_attributes(scanner)?;

    if self_closing {
        // `<script />` — empty section. Allowed.
        return Ok(RawSection {
            open_tag_range: Range::new(open_start, open_end),
            content_range: Range::empty_at(open_end),
            close_tag_range: Range::empty_at(open_end),
            content: "",
            attrs,
        });
    }

    let content_start = open_end;

    // Find the next `</tagname` (case-insensitive). For ASCII-only tag names
    // memchr::memmem::find is case-sensitive, so we do a manual scan.
    let close_pos = match find_close_tag(scanner, close_tag_literal) {
        Some(p) => p,
        None => {
            return Err(ParseError::UnterminatedTag {
                tag_name,
                range: Range::new(open_start, open_end),
            });
        }
    };

    let content_end = close_pos;
    scanner.set_pos(close_pos);
    scanner.advance(close_tag_literal.len() as u32);
    // Swallow any whitespace and the closing `>`.
    scanner.skip_ascii_whitespace();
    if scanner.peek_byte() == Some(b'>') {
        scanner.advance_byte();
    }
    let close_end = scanner.pos();

    let content = &scanner.source()[content_start as usize..content_end as usize];

    Ok(RawSection {
        open_tag_range: Range::new(open_start, open_end),
        content_range: Range::new(content_start, content_end),
        close_tag_range: Range::new(close_pos, close_end),
        content,
        attrs,
    })
}

/// Case-insensitively find the next `</tagname` from the scanner's
/// position. Mirrors upstream's closing regex (`/<\/script\s*>/` in
/// read/script.js): after the tag name only whitespace may precede the
/// `>`, so `</script x` is body text, not a closing tag.
fn find_close_tag(scanner: &Scanner<'_>, close_literal: &str) -> Option<u32> {
    let bytes = scanner.source().as_bytes();
    let start = scanner.pos() as usize;
    let needle = close_literal.as_bytes();
    if needle.is_empty() {
        return None;
    }
    let mut i = start;
    while i + needle.len() <= bytes.len() {
        let window = &bytes[i..i + needle.len()];
        if window
            .iter()
            .zip(needle)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            let mut j = i + needle.len();
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if bytes.get(j) == Some(&b'>') {
                return Some(i as u32);
            }
        }
        i += 1;
    }
    None
}

/// Parse attributes of an opening tag, returning `(attrs, self_closing, end_pos)`.
/// The scanner must be positioned right after the tag name; on return it's
/// positioned just past `>` (or `/>`).
fn parse_tag_attributes(
    scanner: &mut Scanner<'_>,
) -> Result<(Vec<ScriptAttr>, bool, u32), ParseError> {
    let mut attrs = Vec::new();
    let start = scanner.pos();

    loop {
        scanner.skip_ascii_whitespace();
        match scanner.peek_byte() {
            None => {
                // EOF in opening tag.
                return Err(ParseError::MalformedOpenTag {
                    range: Range::new(start - 1, scanner.pos()),
                });
            }
            Some(b'>') => {
                scanner.advance_byte();
                return Ok((attrs, false, scanner.pos()));
            }
            Some(b'/') => {
                scanner.advance_byte();
                scanner.skip_ascii_whitespace();
                if scanner.peek_byte() == Some(b'>') {
                    scanner.advance_byte();
                    return Ok((attrs, true, scanner.pos()));
                }
                // Stray `/` — treat as malformed but don't loop forever.
                return Err(ParseError::MalformedOpenTag {
                    range: Range::new(start - 1, scanner.pos()),
                });
            }
            Some(_) => {
                let attr = parse_one_attr(scanner)?;
                attrs.push(attr);
            }
        }
    }
}

/// Parse a single attribute. Supports: `name`, `name=value`, `name="value"`,
/// `name='value'`. Svelte's `{expr}` shorthand and directives are rare on
/// `<script>`/`<style>` so we don't interpret them here — they get collected
/// as raw text if we see `{`.
fn parse_one_attr(scanner: &mut Scanner<'_>) -> Result<ScriptAttr, ParseError> {
    let start = scanner.pos();
    let name_start = start;

    // Read name: everything up to whitespace, `=`, `>`, `/`.
    while let Some(b) = scanner.peek_byte() {
        if b.is_ascii_whitespace() || matches!(b, b'=' | b'>' | b'/') {
            break;
        }
        scanner.advance_byte();
    }
    if scanner.pos() == name_start {
        return Err(ParseError::MalformedOpenTag {
            range: Range::new(start, scanner.pos().max(start + 1)),
        });
    }
    let name = scanner.source()[name_start as usize..scanner.pos() as usize].to_string();

    // Optional `= value`.
    let saved_before_ws = scanner.pos();
    scanner.skip_ascii_whitespace();
    let value = if scanner.peek_byte() == Some(b'=') {
        scanner.advance_byte();
        scanner.skip_ascii_whitespace();
        Some(parse_attr_value(scanner)?)
    } else {
        scanner.set_pos(saved_before_ws);
        None
    };

    Ok(ScriptAttr {
        name,
        value,
        range: Range::new(start, scanner.pos()),
    })
}

fn parse_attr_value(scanner: &mut Scanner<'_>) -> Result<String, ParseError> {
    let start = scanner.pos();
    match scanner.peek_byte() {
        Some(quote @ (b'"' | b'\'')) => {
            scanner.advance_byte();
            let value_start = scanner.pos();
            while let Some(b) = scanner.peek_byte() {
                if b == quote {
                    let value =
                        scanner.source()[value_start as usize..scanner.pos() as usize].to_string();
                    scanner.advance_byte();
                    return Ok(value);
                }
                scanner.advance_char();
            }
            Err(ParseError::MalformedOpenTag {
                range: Range::new(start, scanner.pos()),
            })
        }
        Some(_) => {
            // Unquoted value — read until whitespace or `>` or `/`.
            let value_start = scanner.pos();
            while let Some(b) = scanner.peek_byte() {
                if b.is_ascii_whitespace() || matches!(b, b'>' | b'/') {
                    break;
                }
                scanner.advance_char();
            }
            Ok(scanner.source()[value_start as usize..scanner.pos() as usize].to_string())
        }
        None => Err(ParseError::MalformedOpenTag {
            range: Range::new(start, scanner.pos()),
        }),
    }
}

fn build_script_section<'src>(
    _source: &'src str,
    raw: RawSection<'src>,
    errors: &mut Vec<ParseError>,
) -> ScriptSection<'src> {
    let context = parse_context_attr(&raw.attrs, errors);
    let pre_err = errors.len();
    let lang = parse_lang_attr(&raw.attrs, errors);
    // Unknown `lang=` (e.g. `<script lang="coffee">`) — upstream's LS
    // `DiagnosticsProvider.ts:72-77` early-returns `[]` for coffee /
    // coffeescript bodies so they never reach TS. Mirror by blanking
    // the body slice: `parse_script_body("", _)` produces an empty
    // AST, no oxc-as-JS parse errors cascade, and the overlay emits
    // only scaffolding. The `UnknownScriptLang` warning that
    // `parse_lang_attr` already pushed remains the user-facing
    // signal that the script is opaque.
    let unknown_lang = errors.len() > pre_err
        && matches!(errors.last(), Some(ParseError::UnknownScriptLang { .. }));
    let (content, content_range) = if unknown_lang {
        (
            "",
            Range::new(raw.content_range.start, raw.content_range.start),
        )
    } else {
        (raw.content, raw.content_range)
    };
    // `generics="T extends ..."` is only meaningful on the INSTANCE
    // script; ignore it on `<script module>` where type parameters
    // wouldn't have anything to apply to (the render function lives in
    // the instance scope).
    let generics = if context == ScriptContext::Instance {
        parse_generics_attr(&raw.attrs)
    } else {
        None
    };
    ScriptSection {
        open_tag_range: raw.open_tag_range,
        content_range,
        close_tag_range: raw.close_tag_range,
        content,
        lang,
        context,
        generics,
        attrs: raw.attrs,
    }
}

fn build_style_section<'src>(_source: &'src str, raw: RawSection<'src>) -> StyleSection<'src> {
    StyleSection {
        open_tag_range: raw.open_tag_range,
        content_range: raw.content_range,
        close_tag_range: raw.close_tag_range,
        content: raw.content,
        attrs: raw.attrs,
    }
}

fn parse_context_attr(attrs: &[ScriptAttr], errors: &mut Vec<ParseError>) -> ScriptContext {
    // Svelte 5 syntax: bare `module` attribute (boolean).
    if attrs
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case("module") && a.value.is_none())
    {
        return ScriptContext::Module;
    }
    // Svelte 4 syntax: `context="module"`.
    let Some(attr) = attrs
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("context"))
    else {
        return ScriptContext::Instance;
    };
    match attr.value.as_deref() {
        Some("module") => ScriptContext::Module,
        Some(other) => {
            errors.push(ParseError::UnknownScriptContext {
                value: other.to_string(),
                range: attr.range,
            });
            ScriptContext::Instance
        }
        None => {
            errors.push(ParseError::UnknownScriptContext {
                value: String::new(),
                range: attr.range,
            });
            ScriptContext::Instance
        }
    }
}

/// Extract the `generics="..."` attribute value. Returns the trimmed
/// string when present and non-empty; `None` otherwise.
fn parse_generics_attr(attrs: &[ScriptAttr]) -> Option<String> {
    let attr = attrs
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("generics"))?;
    let value = attr.value.as_deref()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_lang_attr(attrs: &[ScriptAttr], errors: &mut Vec<ParseError>) -> ScriptLang {
    let Some(attr) = attrs.iter().find(|a| a.name.eq_ignore_ascii_case("lang")) else {
        return ScriptLang::Js;
    };
    match attr.value.as_deref() {
        Some("ts") | Some("typescript") => ScriptLang::Ts,
        Some("js") | Some("javascript") | None => ScriptLang::Js,
        Some("") => ScriptLang::Js,
        Some(other) => {
            errors.push(ParseError::UnknownScriptLang {
                value: other.to_string(),
                range: attr.range,
            });
            ScriptLang::Js
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Document<'_> {
        let (doc, errors) = parse_sections(src);
        assert!(
            errors.is_empty(),
            "expected no errors, got {errors:?} for source:\n{src}"
        );
        doc
    }

    #[test]
    fn in_tag_js_comment_does_not_derail_section_walk() {
        // Svelte 5 allows `//` and `/* */` comments between attributes.
        // A `>` inside such a comment must not end the tag scan early:
        // the comment tail would then re-enter the walk as template
        // text and the real `</div>` would look like a stray closing
        // tag, misrouting the later top-level <script>.
        let src = "<div /* > {#if x} */ class=\"x\">hi</div>\n<script>let a = 1;</script>";
        let doc = parse_ok(src);
        let instance = doc.instance_script.expect("instance script claimed");
        assert_eq!(instance.content, "let a = 1;");
    }

    #[test]
    fn in_tag_line_comment_hides_tag_close_until_newline() {
        let src = "<div // > {#each xs as x}\n  class=\"x\">hi</div>\n<script>let a = 1;</script>";
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_some(), "instance script claimed");
    }

    #[test]
    fn script_close_tag_requires_only_whitespace_before_gt() {
        // Upstream's closing regex is /<\/script\s*>/ — `</script x` is
        // NOT a closing tag, so a body containing that text must not be
        // truncated at it.
        let src = "<script>const s = \"</script x\"; let y = 1;</script>\n{y}";
        let doc = parse_ok(src);
        let instance = doc.instance_script.expect("instance script claimed");
        assert_eq!(instance.content, "const s = \"</script x\"; let y = 1;");
    }

    #[test]
    fn script_close_tag_with_whitespace_still_closes() {
        let src = "<script>let a = 1;</script\n  >";
        let doc = parse_ok(src);
        let instance = doc.instance_script.expect("instance script claimed");
        assert_eq!(instance.content, "let a = 1;");
    }

    #[test]
    fn script_inside_html_comment_not_picked_up_as_instance() {
        // Real-world pattern: a component file keeps its legacy /
        // reference implementation in an HTML comment. Comment content
        // is opaque — treating the inner <script> as the instance
        // script would leak body-scope references into the template
        // and break the overlay.
        let src = r#"<script lang="ts">
import Foo from './Foo.svelte';
</script>
<Foo />
<!-- <script lang="ts">
let position: string;
</script>
{#if position}ignored{/if}
 -->"#;
        let doc = parse_ok(src);
        let s = doc.instance_script.expect("real instance script");
        assert!(
            s.content.contains("import Foo"),
            "expected real script, got: {}",
            s.content
        );
        // There must NOT be a module script synthesized from the
        // commented-out `<script lang="ts">`. `context="module"`
        // isn't set on either, but the important invariant is that
        // we didn't error out on "duplicate script".
    }

    #[test]
    fn style_inside_html_comment_not_picked_up() {
        let src = r#"<style>.a{color:red}</style>
<!-- <style>.b{color:blue}</style> -->"#;
        let doc = parse_ok(src);
        let s = doc.style.expect("real style");
        assert!(s.content.contains(".a"));
        assert!(!s.content.contains(".b"));
    }

    #[test]
    fn void_html_elements_in_template_dont_break_subsequent_style_section() {
        // `<img>` and other HTML void elements have no closing tag, so
        // they must not leave an open element frame behind. A stuck
        // frame would make every following `<style>` block look nested
        // — absorbed into the preceding template run instead of being
        // parsed as a section.
        let src = r#"<script lang="ts">
let x = 1;
</script>
<img src="hero.png" alt="">
<style>.a { color: red }</style>"#;
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_some(), "instance script missing");
        let style = doc.style.expect("style section must be detected");
        assert!(style.content.contains(".a"));
    }

    #[test]
    fn void_html_elements_dont_swallow_following_script_block() {
        // Same shape but for a script block following a void element.
        // Svelte allows scripts at any document position; a stuck
        // element frame would absorb later `<script>` blocks into the
        // template.
        let src = r#"<input type="text">
<script lang="ts">
let y = 2;
</script>"#;
        let doc = parse_ok(src);
        let s = doc
            .instance_script
            .expect("instance script must be detected");
        assert!(s.content.contains("let y"));
    }

    #[test]
    fn empty_source_gives_empty_document() {
        let doc = parse_ok("");
        assert!(doc.module_script.is_none());
        assert!(doc.instance_script.is_none());
        assert!(doc.style.is_none());
        assert!(doc.template.text_runs.is_empty());
    }

    #[test]
    fn template_only_has_one_run() {
        let src = "<h1>hello</h1>";
        let doc = parse_ok(src);
        assert!(doc.module_script.is_none());
        assert_eq!(doc.template.text_runs.len(), 1);
        assert_eq!(doc.template.text_runs[0], Range::new(0, src.len() as u32));
    }

    #[test]
    fn finds_instance_script() {
        let src = r#"<script lang="ts">
let x: number = 1;
</script>
<h1>hi</h1>"#;
        let doc = parse_ok(src);
        let s = doc.instance_script.expect("instance script");
        assert_eq!(s.lang, ScriptLang::Ts);
        assert_eq!(s.context, ScriptContext::Instance);
        assert!(s.content.trim_start().starts_with("let x"));
        // Template is just the bit after </script>.
        assert_eq!(doc.template.text_runs.len(), 1);
    }

    #[test]
    fn finds_module_script() {
        let src = r#"<script context="module">export const hi = 1;</script>"#;
        let doc = parse_ok(src);
        assert!(doc.module_script.is_some());
        assert!(doc.instance_script.is_none());
    }

    #[test]
    fn both_scripts_coexist() {
        let src = r#"<script context="module">export const A = 1;</script>
<script>let b = 2;</script>
<p>hi</p>"#;
        let doc = parse_ok(src);
        assert!(doc.module_script.is_some());
        assert!(doc.instance_script.is_some());
    }

    #[test]
    fn finds_style_block() {
        let src = r#"<h1>hi</h1><style>h1 { color: red; }</style>"#;
        let doc = parse_ok(src);
        let style = doc.style.expect("style");
        assert!(style.content.contains("color: red"));
    }

    #[test]
    fn self_closing_script() {
        let src = r#"<script src="./main.js" />"#;
        let doc = parse_ok(src);
        let s = doc.instance_script.expect("instance script");
        assert_eq!(s.content, "");
    }

    #[test]
    fn case_insensitive_tag_matching() {
        let src = "<SCRIPT>let a = 1;</SCRIPT>";
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_some());
    }

    #[test]
    fn duplicate_instance_script_errors() {
        let src = "<script>let a = 1;</script><script>let b = 2;</script>";
        let (doc, errors) = parse_sections(src);
        assert!(doc.instance_script.is_some());
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ParseError::DuplicateScript { .. }));
    }

    #[test]
    fn duplicate_style_errors() {
        let src = "<style>a{}</style><style>b{}</style>";
        let (_doc, errors) = parse_sections(src);
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ParseError::DuplicateStyle { .. }));
    }

    #[test]
    fn unterminated_script_errors() {
        let src = "<script>let a = 1;";
        let (_doc, errors) = parse_sections(src);
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ParseError::UnterminatedTag { .. }));
    }

    #[test]
    fn lang_attr_parsed() {
        let doc = parse_ok("<script lang=\"ts\">let x:number=1;</script>");
        assert_eq!(doc.instance_script.unwrap().lang, ScriptLang::Ts);

        let doc = parse_ok("<script lang='typescript'>let x:number=1;</script>");
        assert_eq!(doc.instance_script.unwrap().lang, ScriptLang::Ts);

        let doc = parse_ok("<script>let x=1;</script>");
        assert_eq!(doc.instance_script.unwrap().lang, ScriptLang::Js);
    }

    #[test]
    fn script_lang_helper_picks_instance_then_module_then_js() {
        let doc = parse_ok(r#"<script lang="ts">let x:number=1;</script>"#);
        assert_eq!(doc.script_lang(), ScriptLang::Ts);

        let doc = parse_ok("<script>let x=1;</script>");
        assert_eq!(doc.script_lang(), ScriptLang::Js);

        // Module-only script falls back to module's lang.
        let doc = parse_ok(r#"<script context="module" lang="ts">let M=1;</script>"#);
        assert_eq!(doc.script_lang(), ScriptLang::Ts);

        // Neither script tag → JS by default.
        let doc = parse_ok("<div>only template</div>");
        assert_eq!(doc.script_lang(), ScriptLang::Js);

        // Instance script wins over module script.
        let doc = parse_ok(
            r#"<script context="module" lang="ts">let M=1;</script><script>let I=1;</script>"#,
        );
        assert_eq!(doc.script_lang(), ScriptLang::Js);
    }

    #[test]
    fn unknown_lang_emits_error_and_falls_back_to_js() {
        let (doc, errors) = parse_sections(r#"<script lang="coffee">let a = 1;</script>"#);
        assert_eq!(doc.instance_script.unwrap().lang, ScriptLang::Js);
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ParseError::UnknownScriptLang { .. }));
    }

    #[test]
    fn unknown_context_emits_error_and_falls_back_to_instance() {
        let (doc, errors) = parse_sections(r#"<script context="server">x</script>"#);
        assert_eq!(
            doc.instance_script.unwrap().context,
            ScriptContext::Instance
        );
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], ParseError::UnknownScriptContext { .. }));
    }

    #[test]
    fn generics_attr_extracted_on_instance_script() {
        let src = r#"<script lang="ts" generics="T extends { id: string }, K extends keyof T">let x = 1;</script>"#;
        let doc = parse_ok(src);
        let s = doc.instance_script.unwrap();
        assert_eq!(
            s.generics.as_deref(),
            Some("T extends { id: string }, K extends keyof T")
        );
    }

    #[test]
    fn generics_attr_trimmed() {
        // Leading/trailing whitespace around the attribute value is
        // stripped — the parser treats `"  T  "` as `"T"`.
        let doc = parse_ok(r#"<script lang="ts" generics="  T  ">x</script>"#);
        assert_eq!(doc.instance_script.unwrap().generics.as_deref(), Some("T"));
    }

    #[test]
    fn empty_generics_attr_is_none() {
        // `generics=""` is indistinguishable from absence — both yield
        // no-generics emission. Keeping `None` in the field avoids
        // downstream branches guarding against whitespace-only values.
        let doc = parse_ok(r#"<script lang="ts" generics="">x</script>"#);
        assert!(doc.instance_script.unwrap().generics.is_none());
    }

    #[test]
    fn missing_generics_attr_is_none() {
        let doc = parse_ok(r#"<script lang="ts">let x = 1;</script>"#);
        assert!(doc.instance_script.unwrap().generics.is_none());
    }

    #[test]
    fn generics_attr_ignored_on_module_script() {
        // Svelte 5 rejects `<script module generics="T">`; the generic
        // has no render function to apply to. We silently drop it.
        let src = r#"<script module generics="T">export const x = 1;</script>"#;
        let doc = parse_ok(src);
        let ms = doc.module_script.unwrap();
        assert!(
            ms.generics.is_none(),
            "generics on <script module> should be ignored, got {:?}",
            ms.generics
        );
    }

    #[test]
    fn scripted_tag_not_confused_with_script() {
        // A hypothetical <scripted> element should NOT trigger opaque-section.
        let src = "<scripted>body</scripted>";
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_none());
    }

    #[test]
    fn content_range_excludes_tags() {
        let src = "<script>BODY</script>";
        let doc = parse_ok(src);
        let s = doc.instance_script.unwrap();
        assert_eq!(s.content, "BODY");
        assert_eq!(s.content_range.slice(src), "BODY");
    }

    #[test]
    fn attrs_collected_verbatim() {
        let src = r#"<script defer src="x.js" lang="ts">let a = 1;</script>"#;
        let doc = parse_ok(src);
        let s = doc.instance_script.unwrap();
        assert_eq!(s.attrs.len(), 3);
        assert_eq!(s.attrs[0].name, "defer");
        assert_eq!(s.attrs[0].value, None);
        assert_eq!(s.attrs[1].name, "src");
        assert_eq!(s.attrs[1].value.as_deref(), Some("x.js"));
        assert_eq!(s.attrs[2].name, "lang");
        assert_eq!(s.attrs[2].value.as_deref(), Some("ts"));
    }

    #[test]
    fn template_runs_interleave_with_scripts() {
        let src = "before<script>a</script>middle<style>b</style>after";
        let doc = parse_ok(src);
        // Three template runs: "before", "middle", "after".
        assert_eq!(doc.template.text_runs.len(), 3);
        assert_eq!(doc.template.text_runs[0].slice(src), "before");
        assert_eq!(doc.template.text_runs[1].slice(src), "middle");
        assert_eq!(doc.template.text_runs[2].slice(src), "after");
    }

    #[test]
    fn nested_like_text_inside_script_is_opaque() {
        // `</scripting>` shouldn't close `<script>`.
        let src = "<script>let s = '</scripting>'; let t = 1;</script>";
        let doc = parse_ok(src);
        let s = doc.instance_script.unwrap();
        assert!(s.content.contains("</scripting>"));
        assert!(s.content.contains("let t = 1;"));
    }

    // ============================================================
    // Document-level vs nested section decisions.
    //
    // These tests lock the parse shapes we observed breaking real
    // Svelte projects — nested <script> tags inside <svelte:head>
    // templates, mustache expressions with JS comments, template
    // literals containing quote chars, and so on. Each corresponds
    // to a construct a pre-parse scanner once misjudged; the
    // document walk now runs through the template parser itself, so
    // every construct the parser knows (elements, blocks, comments,
    // raw-text bodies, attribute mustaches) is handled by the same
    // grammar that parses the template.
    // ============================================================

    #[test]
    fn nested_script_under_svelte_head_stays_in_template() {
        // Real pattern: an analytics tag inside `<svelte:head>`
        // behind an `{#if}` feature flag. The inner <script> is a
        // regular HTML element and must NOT become the Svelte
        // instance script.
        let src = "<svelte:head>\
                   {#if loaded}\
                   <script defer src=\"https://x\"></script>\
                   {/if}\
                   </svelte:head>";
        let doc = parse_ok(src);
        assert!(
            doc.instance_script.is_none(),
            "nested <script> must not become instance script"
        );
    }

    #[test]
    fn nested_script_with_preceding_top_level_script() {
        // Top-level instance script at the start, analytics script
        // nested inside a template block. The top-level one wins;
        // the nested one is left in the template.
        let src = "<script lang=\"ts\">let x = 1;</script>\n\
                   <svelte:head>{#if loaded}<script src=\"/a.js\"></script>{/if}</svelte:head>";
        let doc = parse_ok(src);
        let s = doc.instance_script.expect("instance script");
        assert!(s.content.contains("let x = 1"));
    }

    #[test]
    fn script_inside_if_block_stays_in_template() {
        // A `<script>` element gated behind a bare `{#if}` — no element
        // wrapper, so only block nesting hides it from document level.
        // The compiler parses this as a regular element (its stack top
        // is the IfBlock, not Root); claiming it as a section would
        // fire a false "duplicate <script> block" error.
        let src = "<script lang=\"ts\">let load = true;</script>\n\
                   {#if load}<script defer src=\"https://x/a.js\"></script>{/if}";
        let doc = parse_ok(src);
        let s = doc.instance_script.expect("instance script");
        assert!(s.content.contains("let load"));
    }

    #[test]
    fn script_inside_if_block_with_expression_attrs() {
        // Expression-valued attributes on a nested script go through
        // the template parser's mustache-aware attribute parser, not
        // the section attribute parser (which reads only plain HTML
        // attribute shapes and would misread `{`).
        let src = "<script lang=\"ts\">let load = true;</script>\n\
                   {#if load}<script defer src={`https://x/lib@${'1.0.0'}/a.js`} \
                   onload={done}></script>{/if}";
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_some());
    }

    #[test]
    fn script_inside_each_await_and_snippet_blocks() {
        // Every block kind nests; `{:then}` / `{:else}` continuations
        // are depth-neutral.
        for src in [
            "{#each items as item}<script src=\"/a.js\"></script>{/each}",
            "{#await p}{:then v}<script src=\"/a.js\"></script>{/await}",
            "{#snippet row()}<script src=\"/a.js\"></script>{/snippet}",
        ] {
            let doc = parse_ok(src);
            assert!(
                doc.instance_script.is_none(),
                "script inside a block must stay in the template: {src}"
            );
        }
    }

    #[test]
    fn top_level_script_after_closed_block_is_claimed() {
        // Scripts may appear anywhere at document level, including
        // after template content; once a block closes the walk is back
        // at the document root.
        let src = "{#if visible}<p>hi</p>{/if}\n<script>let x = 1;</script>";
        let doc = parse_ok(src);
        let s = doc.instance_script.expect("instance script after block");
        assert!(s.content.contains("let x = 1"));
    }

    #[test]
    fn stray_block_close_does_not_hide_later_script() {
        // An unbalanced `{/if}` is a stray terminator, not an open
        // scope — it must not permanently hide later document-level
        // sections.
        let src = "{/if}\n<script>let x = 1;</script>";
        let (doc, _errors) = parse_sections(src);
        assert!(doc.instance_script.is_some());
    }

    #[test]
    fn nested_script_body_is_raw_text() {
        // The nested body is raw text; `<` and `{` inside the JS must
        // not open phantom elements or mustaches, or the following
        // top-level <style> would be absorbed into the template.
        let src = "{#if x}<script>if (a<b) { run('</div>'); }</script>{/if}\n\
                   <style>p { color: red }</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some(), "style after nested script body");
    }

    #[test]
    fn self_closing_nested_script_inside_block() {
        let src = "{#if x}<script src=\"/a.js\" />{/if}<script>let y = 2;</script>";
        let doc = parse_ok(src);
        let s = doc.instance_script.expect("instance script");
        assert!(s.content.contains("let y = 2"));
    }

    #[test]
    fn whitespace_padded_close_tag_still_closes_block() {
        // The compiler allows whitespace after `{` in block tags;
        // `{ /if}` must close the block like `{/if}` so the following
        // script sits back at document level.
        let src = "{#if x}<p>hi</p>{ /if}\n<script>let x = 1;</script>";
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_some());
    }

    #[test]
    fn nested_style_inside_div_stays_in_template() {
        let src = "<div><style>p { color: red; }</style></div>";
        let doc = parse_ok(src);
        assert!(
            doc.style.is_none(),
            "nested <style> inside <div> isn't the Svelte style section"
        );
    }

    #[test]
    fn top_level_style_after_closed_div_still_becomes_section() {
        // `<div>hi</div><style>…</style>` is the common pattern — the
        // closed div leaves the walk back at document level, so the
        // <style> is grabbed.
        let src = "<div>hi</div><style>p { color: red; }</style>";
        let doc = parse_ok(src);
        let s = doc.style.expect("style section");
        assert!(s.content.contains("color: red"));
    }

    #[test]
    fn self_closing_svelte_options_keeps_document_level() {
        // `<svelte:options runes />` is self-closing and conventionally
        // precedes the script; it must not leave a frame open.
        let src = "<svelte:options runes />\n<script>let x = 1;</script>";
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_some());
    }

    #[test]
    fn self_closing_br_in_template_still_allows_later_style() {
        // Void-ish / self-closed tags never wait for a closing tag.
        let src = "<div><br /></div><style>p{}</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some());
    }

    #[test]
    fn mustache_with_lt_gt_operators_does_not_confuse_depth() {
        // `{a < b}` and `{a > b}` inside interpolations are operator
        // tokens, not tag markers — treating `<b>` inside an
        // expression as an opening tag would consume source through
        // the next `>` and derail everything after it.
        let src = "<div>{a < b}</div><style>p{}</style>";
        let doc = parse_ok(src);
        assert!(
            doc.style.is_some(),
            "style must be grabbed after the mustache"
        );
    }

    #[test]
    fn mustache_with_apostrophe_in_line_comment_does_not_run_off() {
        // An apostrophe in `// don't` inside an attribute-value
        // mustache must read as comment text, not a string opener — a
        // phantom string that never closes would eat every subsequent
        // `{` and `}` and leave the template runs misaligned.
        let src = "<div on:click={() => {\n\
                   // We don't close this comment with a quote\n\
                   doSomething();\n\
                   }}>x</div>\n\
                   <style>p { color: red; }</style>";
        let doc = parse_ok(src);
        let s = doc
            .style
            .expect("style after a commented-apostrophe mustache");
        assert!(s.content.contains("color: red"));
    }

    #[test]
    fn mustache_with_block_comment_and_braces() {
        // Block comments `/* … */` inside expressions containing
        // braces must not flip mustache depth.
        let src = "<div onclick={() => { /* { don't */ run(); }}>x</div>\n\
                   <style>p{}</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some());
    }

    #[test]
    fn mustache_template_literal_with_dollar_braces() {
        // `` `${x}` `` template literals contain an inner `${` that
        // must not be counted as a mustache-open. Also apostrophes
        // inside ordinary template-literal text must not start a
        // string scan.
        let src = "<div title={`don't ${a} end`}>x</div>\n\
                   <style>p{}</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some());
    }

    #[test]
    fn mustache_with_object_literal_in_attr_value() {
        // `use:dndzone={{ dragDisabled: !x }}` — outer `{` is
        // mustache, inner `{` is an object literal; both must unwind
        // before the attribute value ends.
        let src = "<div use:dndzone={{ dragDisabled: !x, items: [1, 2] }}>y</div>\n\
                   <style>p{}</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some());
    }

    #[test]
    fn mustache_with_regex_containing_brace() {
        // `/}/ ` — a regex literal whose body is a `}` — must not end
        // the mustache early; closing at the `}` inside the regex
        // would desync the walk and miss the trailing `<style>`.
        let src = "<div title={x.replace(/}/g, '')}>y</div>\n\
                   <style>p{}</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some());
    }

    #[test]
    fn mustache_with_nested_if_block() {
        // A balanced `{#if}` / `{/if}` pair returns the walk to
        // document level, so the trailing `<style>` is a section.
        let src = "{#if ready}<div>hi</div>{/if}<style>p{}</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some());
    }

    #[test]
    fn script_with_generic_attr_and_quoted_value() {
        // `<script lang="ts" generics="S">` — `"` in attribute
        // values must stay inside the attribute, not leak into the
        // surrounding tag scan.
        let src = "<script lang=\"ts\" generics=\"S\">let x: S;</script>\n\
                   <div>hi</div>\n\
                   <style>p{}</style>";
        let doc = parse_ok(src);
        let s = doc.instance_script.expect("instance");
        assert_eq!(s.generics.as_deref(), Some("S"));
        assert!(doc.style.is_some());
    }

    #[test]
    fn unmatched_closing_tag_doesnt_hide_later_script() {
        // `</div>` without a matching `<div>` is a stray closing tag,
        // not an enclosing scope — the script after it is still at
        // document level.
        let src = "</div><script>let x = 1;</script>";
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_some());
    }
}
