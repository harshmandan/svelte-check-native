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
    // Tag-nesting depth. `<script>` / `<style>` are grabbed as Svelte
    // sections only when depth == 0; nested occurrences (analytics
    // snippet under `<svelte:head>{#if}…`) are left for the template
    // parser. Tracked by a cheap `<NAME>` / `</NAME>` counter that
    // respects quoted attribute values and self-closing tags. Not a
    // real DOM — just enough to distinguish "document level" from
    // "inside something."
    let mut tag_depth: u32 = 0;

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

        if tag_depth == 0
            && scanner.peek_byte() == Some(b'<')
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

        if tag_depth == 0
            && scanner.peek_byte() == Some(b'<')
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

        // Mustache — skip the whole balanced `{…}` region. `<` and
        // `>` inside a mustache expression (`{a < b}`) are operator
        // tokens, not tag markers, and must not perturb tag_depth.
        // Skip quickly by counting braces with string-literal
        // awareness (`'…'`, `"…"`, template `` `…` `` so a `{` inside
        // a string doesn't increment depth).
        if scanner.peek_byte() == Some(b'{') {
            scanner.set_pos(scan_past_mustache(
                scanner.source().as_bytes(),
                scanner.pos(),
            ));
            continue;
        }
        // Track `<NAME>` / `</NAME>` nesting — cheap but sufficient
        // for "is a following `<script>` at document level?".
        if scanner.peek_byte() == Some(b'<') {
            let bytes = scanner.source().as_bytes();
            let pos = scanner.pos() as usize;
            let next = bytes.get(pos + 1).copied();
            if next == Some(b'/') {
                tag_depth = tag_depth.saturating_sub(1);
                let mut i = pos + 2;
                while i < bytes.len() && bytes[i] != b'>' {
                    i += 1;
                }
                scanner.set_pos((i + 1).min(bytes.len()) as u32);
                continue;
            }
            if next.is_some_and(|b| b.is_ascii_alphabetic()) {
                let (end, self_closing) = scan_past_open_tag(bytes, pos);
                if !self_closing {
                    tag_depth += 1;
                }
                scanner.set_pos(end as u32);
                continue;
            }
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

/// Skip past a balanced `{…}` mustache block at `from`, returning
/// the index just past the matching `}`. Understands enough JS
/// lexical structure to not get fooled by braces / quotes inside
/// strings, template literals, or comments:
///
/// - `'…'`, `"…"`, `` `…` `` respect backslash escapes; embedded
///   braces don't count.
/// - `// … \n` and `/* … */` are skipped entirely, so an apostrophe
///   inside `// don't` doesn't start a phantom string.
///
/// On EOF, returns the source length.
fn scan_past_mustache(bytes: &[u8], from: u32) -> u32 {
    let mut i = from as usize + 1;
    let mut depth: u32 = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                // Line comment — scan to end of line (or EOF).
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                // Block comment — scan to `*/`.
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            b'\\' if i + 1 < bytes.len() => {
                i += 2;
                continue;
            }
            b'"' | b'\'' | b'`' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
                if depth == 0 {
                    return i as u32;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len() as u32
}

/// Skip past `<NAME … >` or `<NAME … />` starting at `from`. Returns
/// `(index_past_close, treat_as_zero_depth)`. The boolean is true
/// when the tag should not increment template tag-depth: an explicit
/// `/>` self-close OR an HTML void element (`<img>`, `<br>`, etc.)
/// which by spec has no closing tag. Respects quoted attribute
/// values and balanced mustache regions.
fn scan_past_open_tag(bytes: &[u8], from: usize) -> (usize, bool) {
    let mut i = from + 1;
    // Extract the tag name so we can recognize HTML void elements.
    // Stops at the first byte that can't be part of a tag name
    // (whitespace, `/`, `>`, attribute boundary).
    let name_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'-' | b':')) {
        i += 1;
    }
    let is_void = is_void_html_tag(&bytes[name_start..i]);
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
            b'{' => {
                // Attribute value with mustache — skip balanced.
                i = scan_past_mustache(bytes, i as u32) as usize;
            }
            b'/' if bytes.get(i + 1) == Some(&b'>') => {
                return (i + 2, true);
            }
            b'>' => return (i + 1, is_void),
            _ => i += 1,
        }
    }
    (bytes.len(), is_void)
}

/// HTML5 void elements (https://html.spec.whatwg.org/#void-elements).
/// These never have a closing tag and so must not push our top-level
/// section walker into an apparent-nesting state. Without this check
/// a `<style>` block following a void element gets absorbed into the
/// preceding template run instead of recognised as a section.
///
/// Svelte requires lowercase HTML element names so we match
/// case-sensitively; tag-name byte slices come straight from source.
fn is_void_html_tag(name: &[u8]) -> bool {
    matches!(
        name,
        b"area"
            | b"base"
            | b"br"
            | b"col"
            | b"embed"
            | b"hr"
            | b"img"
            | b"input"
            | b"keygen"
            | b"link"
            | b"meta"
            | b"param"
            | b"source"
            | b"track"
            | b"wbr"
    )
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
    fn void_html_elements_in_template_dont_break_subsequent_style_section() {
        // Regression: `<img>` and other HTML void elements have no
        // closing tag, so our top-level walker must not treat them as
        // pushing tag depth. Without the void-element list, depth
        // never returns to zero after a void element appears in the
        // template, and a following `<style>` block gets absorbed
        // into the preceding template run instead of being parsed as
        // a section.
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
        // Same regression but for a script block following a void
        // element. Svelte allows scripts at any document position;
        // the void-element bug also caused later `<script>` blocks to
        // be absorbed into the template.
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
    // Tag-depth tracking + mustache-aware scan tests.
    //
    // These tests lock the parse shapes we observed breaking real
    // Svelte projects — nested <script> tags inside <svelte:head>
    // templates, mustache expressions with JS comments, template
    // literals containing quote chars, and so on. The mustache
    // scanner went through several iterations (missing JS-comment
    // handling, off-by-one on self-closing tags); each test below
    // corresponds to a class of bug we hit once and want to block.
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
        // `<div>hi</div><style>…</style>` is the common pattern —
        // depth returns to 0 after the </div> so <style> is grabbed.
        let src = "<div>hi</div><style>p { color: red; }</style>";
        let doc = parse_ok(src);
        let s = doc.style.expect("style section");
        assert!(s.content.contains("color: red"));
    }

    #[test]
    fn self_closing_svelte_options_keeps_document_level() {
        // `<svelte:options runes />` is self-closing and conventionally
        // precedes the script. Depth must stay 0 after it.
        let src = "<svelte:options runes />\n<script>let x = 1;</script>";
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_some());
    }

    #[test]
    fn self_closing_br_in_template_still_allows_later_style() {
        // Void-ish / self-closed tags don't accumulate depth.
        let src = "<div><br /></div><style>p{}</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some());
    }

    #[test]
    fn mustache_with_lt_gt_operators_does_not_confuse_depth() {
        // `{a < b}` and `{a > b}` inside interpolations should not
        // flip tag_depth. Without mustache-aware skipping we
        // previously treated `<b>` inside an expression as an
        // opening tag and consumed source through the next `>`,
        // producing wildly wrong parse results downstream.
        let src = "<div>{a < b}</div><style>p{}</style>";
        let doc = parse_ok(src);
        assert!(
            doc.style.is_some(),
            "style must be grabbed after the mustache"
        );
    }

    #[test]
    fn mustache_with_apostrophe_in_line_comment_does_not_run_off() {
        // The bug that landed 15 new TS parse errors on a
        // real-world bench: an apostrophe in `// don't` inside an
        // attribute-value mustache opened a phantom string that
        // never closed. Every subsequent `{` and `}` was eaten as
        // part of the "string" and tag_depth desynced, leaving
        // template runs misaligned.
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
        // mustache, inner `{` is an object literal. Depth counter
        // correctly unwinds both.
        let src = "<div use:dndzone={{ dragDisabled: !x, items: [1, 2] }}>y</div>\n\
                   <style>p{}</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some());
    }

    #[test]
    fn mustache_with_nested_if_block() {
        // `{#if a}` / `{/if}` are Svelte block tags — each a
        // balanced `{…}`. The sections scanner doesn't need to
        // understand the block semantics, just count braces.
        let src = "{#if ready}<div>hi</div>{/if}<style>p{}</style>";
        let doc = parse_ok(src);
        assert!(doc.style.is_some());
    }

    #[test]
    fn script_with_generic_attr_and_quoted_value() {
        // `<script lang="ts" generics="S">` — `"` in attribute
        // values must not be misread as tag-name chars or leak
        // depth tracking.
        let src = "<script lang=\"ts\" generics=\"S\">let x: S;</script>\n\
                   <div>hi</div>\n\
                   <style>p{}</style>";
        let doc = parse_ok(src);
        let s = doc.instance_script.expect("instance");
        assert_eq!(s.generics.as_deref(), Some("S"));
        assert!(doc.style.is_some());
    }

    #[test]
    fn unmatched_closing_tag_doesnt_underflow_depth() {
        // `</div>` without a matching `<div>` — depth counter must
        // saturate at 0, not wrap.
        let src = "</div><script>let x = 1;</script>";
        let doc = parse_ok(src);
        assert!(doc.instance_script.is_some());
    }
}
