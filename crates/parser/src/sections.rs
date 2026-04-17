//! Structural top-level parser.
//!
//! Walks the source and identifies top-level `<script>`, `<script context="module">`,
//! and `<style>` sections. Everything not inside one of those is template text.
//!
//! Inside a script or style block, HTML rules apply: everything up to the
//! matching `</script>` / `</style>` (case-insensitive) is opaque — we don't
//! interpret strings, comments, or template literals. This matches browser
//! HTML parsing (and Svelte's own parser).
//!
//! The body of script blocks is recovered verbatim so oxc can parse it; the
//! body of style blocks is recovered for future CSS validation.

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
pub fn parse_sections(source: &str) -> (Document<'_>, Vec<ParseError>) {
    let mut scanner = Scanner::new(source);
    let mut errors: Vec<ParseError> = Vec::new();

    let mut module_script: Option<ScriptSection<'_>> = None;
    let mut instance_script: Option<ScriptSection<'_>> = None;
    let mut style: Option<StyleSection<'_>> = None;

    let mut template_runs: Vec<Range> = Vec::new();
    let mut template_cursor: u32 = 0;

    while !scanner.eof() {
        let here = scanner.pos();

        // We only interpret `<script` / `<style` at positions where they
        // look like tag starts: the preceding char should be '>' or
        // whitespace or BOF so we don't misfire inside attribute values.
        // In practice these identifiers only appear at the top level of
        // a Svelte file (or inside templates as string literals), and the
        // Svelte compiler treats them as opaque-tag triggers whenever they
        // appear at a `<` position. Matching upstream behavior: any '<' that
        // starts with `<script` or `<style` (case-insensitive) becomes an
        // opaque section.

        // HTML comment content is opaque — must not be scanned for
        // `<script>` / `<style>` tags. Real-world pattern: a
        // component's reference / legacy code kept as commented-out
        // `<!-- <script>...</script>...{#if x}...-->`. Without this
        // skip, the sections pass picks up the inner `<script>` as
        // the instance script and the template parser picks up inner
        // `{#if}` blocks referencing names that only exist in the
        // commented-out code, firing TS2304 in the overlay.
        if scanner.peek_byte() == Some(b'<') && scanner.starts_with("<!--") {
            let after_open = scanner.pos() as usize + 4;
            match scanner.source()[after_open..].find("-->") {
                Some(offset) => {
                    let skip_to = (after_open + offset + 3).min(scanner.source().len());
                    scanner.set_pos(skip_to as u32);
                }
                None => {
                    // Unterminated comment — treat rest of source as
                    // comment body to avoid re-scanning the same `<!--`.
                    // parse_template below will surface the unterminated
                    // error when it walks the same region.
                    scanner.set_pos(scanner.source().len() as u32);
                }
            }
            continue;
        }

        if scanner.peek_byte() == Some(b'<')
            && (scanner.starts_with_ignore_case("<script")
                && !is_ident_char(scanner.peek_byte_at(7)))
        {
            // Flush pending template text.
            if template_cursor < here {
                template_runs.push(Range::new(template_cursor, here));
            }
            match parse_opaque_section(&mut scanner, "script") {
                Ok(raw) => {
                    let section = build_script_section(source, raw, &mut errors);
                    let is_module = section.context == ScriptContext::Module;
                    let open_range = section.open_tag_range;
                    let close_range = section.close_tag_range;
                    let is_duplicate_script = if is_module {
                        let dup = module_script.is_some();
                        if dup {
                            errors.push(ParseError::DuplicateScript {
                                descriptor: " context=\"module\"",
                                range: open_range,
                            });
                        } else {
                            module_script = Some(section);
                        }
                        dup
                    } else if instance_script.is_some() {
                        errors.push(ParseError::DuplicateScript {
                            descriptor: "",
                            range: open_range,
                        });
                        true
                    } else {
                        instance_script = Some(section);
                        false
                    };
                    // A "duplicate" script is almost always a `<script>`
                    // element that lives INSIDE the template (typically
                    // nested under `<svelte:head>` for analytics / Google
                    // Identity Services tags). Its opening-tag attributes
                    // often reference script-local bindings — e.g.
                    // `onload={useManualGoogleAuth('signin')}` — which
                    // must be scanned by the template-ref pass so the
                    // import isn't flagged as TS6133 "declared but never
                    // read". Add the opening tag's span to the template
                    // runs; the template parser then picks it up as
                    // normal element content and its attribute expressions
                    // flow through the usual walker.
                    if is_duplicate_script {
                        template_runs.push(Range::new(open_range.start, open_range.end));
                        if close_range.start < close_range.end {
                            template_runs.push(Range::new(close_range.start, close_range.end));
                        }
                    }
                    template_cursor = scanner.pos();
                }
                Err(err) => {
                    errors.push(err);
                    // Skip the `<` we just saw so we don't loop forever.
                    scanner.advance_byte();
                }
            }
            continue;
        }

        if scanner.peek_byte() == Some(b'<')
            && (scanner.starts_with_ignore_case("<style")
                && !is_ident_char(scanner.peek_byte_at(6)))
        {
            if template_cursor < here {
                template_runs.push(Range::new(template_cursor, here));
            }
            match parse_opaque_section(&mut scanner, "style") {
                Ok(raw) => {
                    let section = build_style_section(source, raw);
                    if style.is_some() {
                        errors.push(ParseError::DuplicateStyle {
                            range: section.open_tag_range,
                        });
                    } else {
                        style = Some(section);
                    }
                    template_cursor = scanner.pos();
                }
                Err(err) => {
                    errors.push(err);
                    scanner.advance_byte();
                }
            }
            continue;
        }

        // Anything else: keep walking.
        scanner.advance_char();
    }

    // Flush trailing template text.
    if template_cursor < scanner.pos() {
        template_runs.push(Range::new(template_cursor, scanner.pos()));
    }

    let doc = Document {
        source,
        module_script,
        instance_script,
        style,
        template: Template {
            text_runs: template_runs,
        },
    };
    (doc, errors)
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
    // Eat `<tagname` (case-insensitive).
    let lead = format!("<{tag_name}");
    debug_assert!(
        scanner.starts_with_ignore_case(&lead),
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
    let close_tag_literal = format!("</{tag_name}");

    // Find the next `</tagname` (case-insensitive). For ASCII-only tag names
    // memchr::memmem::find is case-sensitive, so we do a manual scan.
    let close_pos = match find_close_tag(scanner, &close_tag_literal) {
        Some(p) => p,
        None => {
            let tag_name_static: &'static str = match tag_name {
                "script" => "script",
                "style" => "style",
                _ => "unknown",
            };
            return Err(ParseError::UnterminatedTag {
                tag_name: tag_name_static,
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

/// Case-insensitively find the next `</tagname` from the scanner's position.
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
            // Sanity: ensure next byte isn't an ident continuation, else
            // `</scripted>` would match `</script`. Check the char right
            // after the literal.
            match bytes.get(i + needle.len()).copied() {
                None => return Some(i as u32),
                Some(next) => {
                    if !(next.is_ascii_alphanumeric() || matches!(next, b'-' | b'_')) {
                        return Some(i as u32);
                    }
                }
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
    let lang = parse_lang_attr(&raw.attrs, errors);
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
        content_range: raw.content_range,
        close_tag_range: raw.close_tag_range,
        content: raw.content,
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
    fn script_inside_html_comment_not_picked_up_as_instance() {
        // Real-world pattern: a component file keeps its legacy /
        // reference implementation in an HTML comment. Without the
        // comment-skip, the sections pass wires up the inner
        // <script> as the instance script, which leaks body-scope
        // references into the template and breaks the overlay.
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
}
