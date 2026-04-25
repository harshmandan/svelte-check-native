//! Attribute and directive parsing.
//!
//! Called from [`crate::template`] immediately after the tag name of an
//! opening tag. Leaves the scanner positioned just past the final attribute
//! — the caller handles `>` / `/>`.
//!
//! ### What's covered
//!
//! - Plain attributes: `name`, `name="value"`, `name='value'`, `name=value`
//! - Expression attributes: `name={expr}`
//! - Shorthand: `{name}`
//! - Spread: `{...expr}`
//! - Directives with the prefixes: `bind`, `on`, `use`, `class`, `style`,
//!   `transition`, `in`, `out`, `animate`, `let`
//! - Directive modifiers: `on:click|once|preventDefault={handler}`
//! - Bind getter/setter pair: `bind:value={getter, setter}`
//!
//! ### What's NOT covered yet
//!
//! - **Interpolations inside quoted attribute values** (e.g. `class="a {b} c"`).
//!   Quoted values are currently stored as a single literal Text part. Real
//!   parsing of `{expr}` islands inside quoted strings — needed so that
//!   identifiers like `b` inside `style:left="{b}px"` are seen as
//!   referenced — is not yet implemented.

use smol_str::SmolStr;
use svn_core::Range;

use crate::ast::{
    AttrValue, AttrValuePart, Attribute, Directive, DirectiveKind, DirectiveValue, ExpressionAttr,
    PlainAttr, ShorthandAttr, SpreadAttr,
};
use crate::error::ParseError;
use crate::mustache::find_mustache_end;
use crate::scanner::Scanner;

/// Parse zero or more attributes, stopping before `>` or `/>`.
///
/// On return, the scanner points at the `>` or `/` of the tag close.
pub fn parse_attributes(
    scanner: &mut Scanner<'_>,
    fragment_end: u32,
    errors: &mut Vec<ParseError>,
) -> Vec<Attribute> {
    let mut attrs = Vec::new();

    loop {
        scanner.skip_ascii_whitespace();

        if scanner.pos() >= fragment_end || scanner.eof() {
            // Unterminated tag; let the caller report the higher-level error.
            return attrs;
        }

        match scanner.peek_byte() {
            Some(b'>') | Some(b'/') | None => return attrs,
            Some(b'{') => {
                if let Some(attr) = parse_brace_attribute(scanner, errors) {
                    attrs.push(attr);
                }
            }
            Some(_) => {
                if let Some(attr) = parse_named_attribute(scanner, errors) {
                    attrs.push(attr);
                } else {
                    // Recovery: advance one byte to avoid infinite loops.
                    scanner.advance_byte();
                }
            }
        }
    }
}

// ===== Brace-prefixed attributes: `{name}` and `{...expr}` ===============

fn parse_brace_attribute(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
) -> Option<Attribute> {
    let start = scanner.pos();
    debug_assert_eq!(scanner.peek_byte(), Some(b'{'));
    scanner.advance_byte();

    // Inside the brace — skip leading whitespace.
    let content_start = scanner.pos();
    let mut cursor = Scanner::new(scanner.source());
    cursor.set_pos(content_start);
    cursor.skip_ascii_whitespace();

    if cursor.starts_with("...") {
        // Spread: `{...expr}`.
        cursor.advance(3);
        let expr_start = cursor.pos();
        let end = match find_mustache_end(scanner.source(), expr_start) {
            Some(e) => e,
            None => {
                errors.push(ParseError::UnterminatedMustache {
                    range: Range::new(start, scanner.source().len() as u32),
                });
                return None;
            }
        };
        scanner.set_pos(end + 1);
        return Some(Attribute::Spread(SpreadAttr {
            expression_range: Range::new(expr_start, end),
            range: Range::new(start, end + 1),
            is_attach: false,
        }));
    }

    if cursor.peek_byte() == Some(b'@') {
        // `{@attach fn(...)}` (Svelte 5.29+) — the `@attach` tag in
        // attribute position. The body is an expression that returns an
        // attachment. We don't model attachments at the type level; we
        // expose the body as a spread-like expression so the template-ref
        // pass can walk it for identifier references (otherwise things
        // like `{@attach floating({offset, shift, flip})}` lose the
        // import refs).
        cursor.advance_byte(); // past `@`
        // Skip the tag keyword.
        while let Some(b) = cursor.peek_byte() {
            if b.is_ascii_alphabetic() {
                cursor.advance_byte();
            } else {
                break;
            }
        }
        cursor.skip_ascii_whitespace();
        let expr_start = cursor.pos();
        let end = match find_mustache_end(scanner.source(), expr_start) {
            Some(e) => e,
            None => {
                errors.push(ParseError::UnterminatedMustache {
                    range: Range::new(start, scanner.source().len() as u32),
                });
                return None;
            }
        };
        scanner.set_pos(end + 1);
        return Some(Attribute::Spread(SpreadAttr {
            expression_range: Range::new(expr_start, end),
            range: Range::new(start, end + 1),
            is_attach: true,
        }));
    }

    // Shorthand: `{name}`. Read identifier, require `}` next (optionally
    // after whitespace).
    let name_start = cursor.pos();
    while let Some(b) = cursor.peek_byte() {
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
            cursor.advance_byte();
        } else {
            break;
        }
    }
    let name_end = cursor.pos();

    cursor.skip_ascii_whitespace();
    if cursor.peek_byte() != Some(b'}') || name_end == name_start {
        // Not a plain `{name}` — reuse the mustache scanner to find the end
        // and report an "unsupported attribute form" (e.g. inline
        // expression). Actually, Svelte doesn't accept bare expressions as
        // attributes — so treat as error.
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(start, scanner.pos()),
        });
        // Try to recover by skipping to the matching `}`.
        if let Some(e) = find_mustache_end(scanner.source(), content_start) {
            scanner.set_pos(e + 1);
        } else {
            scanner.advance_byte();
        }
        return None;
    }

    let close_brace_pos = cursor.pos();
    scanner.set_pos(close_brace_pos + 1);

    let name: SmolStr = scanner.source()[name_start as usize..name_end as usize].into();
    Some(Attribute::Shorthand(ShorthandAttr {
        name,
        range: Range::new(start, scanner.pos()),
    }))
}

// ===== Named attributes (plain, expression, directive) ===================

fn parse_named_attribute(
    scanner: &mut Scanner<'_>,
    errors: &mut Vec<ParseError>,
) -> Option<Attribute> {
    let start = scanner.pos();

    // Read the attribute name. Directive names include `:`; HTML attrs can
    // include hyphens, digits, underscores, and (rarely) `$`. `|` is part
    // of directive-modifier syntax (`on:click|once`) — valid only after a
    // `:` but harmless to allow generally since plain HTML attrs don't use
    // pipes.
    let name_start = scanner.pos();
    while let Some(b) = scanner.peek_byte() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b':' | b'.' | b'$' | b'|') {
            scanner.advance_byte();
        } else {
            break;
        }
    }
    if scanner.pos() == name_start {
        return None;
    }
    let name = &scanner.source()[name_start as usize..scanner.pos() as usize];

    // Split directive prefix.
    let directive_prefix = name
        .find(':')
        .and_then(|idx| DirectiveKind::parse(&name[..idx]).map(|k| (k, idx)));

    if let Some((kind, colon_idx)) = directive_prefix {
        return parse_directive(scanner, start, kind, colon_idx, errors);
    }

    // Plain-style named attribute.
    let name_sym: SmolStr = name.into();
    let value = if scanner.peek_byte() == Some(b'=') {
        scanner.advance_byte();
        match parse_attr_value(scanner, errors) {
            Some(v) => Some(v),
            None => return None,
        }
    } else {
        None
    };

    // If the value came through as a single expression (no text parts), we
    // distinguish Plain vs Expression attributes for convenience of
    // downstream consumers.
    if let Some(AttrValue {
        parts,
        quoted,
        range,
    }) = &value
    {
        if !quoted && parts.len() == 1 {
            if let AttrValuePart::Expression {
                expression_range, ..
            } = parts[0]
            {
                return Some(Attribute::Expression(ExpressionAttr {
                    name: name_sym,
                    expression_range,
                    range: Range::new(start, range.end),
                }));
            }
        }
    }

    let end = value
        .as_ref()
        .map(|v| v.range.end)
        .unwrap_or_else(|| scanner.pos());
    Some(Attribute::Plain(PlainAttr {
        name: name_sym,
        value,
        range: Range::new(start, end),
    }))
}

fn parse_directive(
    scanner: &mut Scanner<'_>,
    start: u32,
    kind: DirectiveKind,
    colon_idx: usize,
    errors: &mut Vec<ParseError>,
) -> Option<Attribute> {
    // The name and modifiers were consumed as part of the name scan (they
    // contain no `=` or whitespace). Parse them out of the collected slice.
    let full_name = &scanner.source()[(start as usize) + colon_idx + 1..scanner.pos() as usize];
    let mut parts = full_name.split('|');
    let dir_name: SmolStr = parts.next().unwrap_or("").into();
    let modifiers: Vec<SmolStr> = parts.map(SmolStr::from).collect();

    if dir_name.is_empty() {
        errors.push(ParseError::MalformedOpenTag {
            range: Range::new(start, scanner.pos()),
        });
        return None;
    }

    let value = if scanner.peek_byte() == Some(b'=') {
        scanner.advance_byte();
        Some(parse_directive_value(scanner, kind, errors)?)
    } else {
        None
    };

    let end = value
        .as_ref()
        .map(|v| match v {
            DirectiveValue::Expression { range, .. } => range.end,
            DirectiveValue::BindPair { range, .. } => range.end,
            DirectiveValue::Quoted(v) => v.range.end,
        })
        .unwrap_or_else(|| scanner.pos());

    Some(Attribute::Directive(Directive {
        kind,
        name: dir_name,
        modifiers,
        value,
        range: Range::new(start, end),
    }))
}

fn parse_directive_value(
    scanner: &mut Scanner<'_>,
    kind: DirectiveKind,
    errors: &mut Vec<ParseError>,
) -> Option<DirectiveValue> {
    match scanner.peek_byte() {
        Some(b'{') => {
            let start = scanner.pos();
            scanner.advance_byte();
            let expr_start = scanner.pos();
            let end = match find_mustache_end(scanner.source(), expr_start) {
                Some(e) => e,
                None => {
                    errors.push(ParseError::UnterminatedMustache {
                        range: Range::new(start, scanner.source().len() as u32),
                    });
                    return None;
                }
            };
            scanner.set_pos(end + 1);

            // For bind:foo={getter, setter}, detect a top-level comma.
            if matches!(kind, DirectiveKind::Bind) {
                if let Some(comma_pos) = find_top_level_comma(scanner.source(), expr_start, end) {
                    return Some(DirectiveValue::BindPair {
                        getter_range: Range::new(expr_start, comma_pos),
                        setter_range: Range::new(comma_pos + 1, end),
                        range: Range::new(start, end + 1),
                    });
                }
            }

            Some(DirectiveValue::Expression {
                expression_range: Range::new(expr_start, end),
                range: Range::new(start, end + 1),
            })
        }
        Some(b'"') | Some(b'\'') => {
            let quoted = parse_quoted_value(scanner, errors)?;
            Some(DirectiveValue::Quoted(quoted))
        }
        _ => {
            // Unquoted directive values aren't standard; treat as a plain
            // text run for now.
            let start = scanner.pos();
            let text_start = start;
            while let Some(b) = scanner.peek_byte() {
                if b.is_ascii_whitespace() || matches!(b, b'>' | b'/') {
                    break;
                }
                scanner.advance_char();
            }
            let end = scanner.pos();
            let content = scanner.source()[text_start as usize..end as usize].to_string();
            Some(DirectiveValue::Quoted(AttrValue {
                parts: vec![AttrValuePart::Text {
                    content,
                    range: Range::new(text_start, end),
                }],
                range: Range::new(start, end),
                quoted: false,
            }))
        }
    }
}

fn parse_attr_value(scanner: &mut Scanner<'_>, errors: &mut Vec<ParseError>) -> Option<AttrValue> {
    match scanner.peek_byte()? {
        b'"' | b'\'' => parse_quoted_value(scanner, errors),
        b'{' => {
            let start = scanner.pos();
            scanner.advance_byte();
            let expr_start = scanner.pos();
            let end = match find_mustache_end(scanner.source(), expr_start) {
                Some(e) => e,
                None => {
                    errors.push(ParseError::UnterminatedMustache {
                        range: Range::new(start, scanner.source().len() as u32),
                    });
                    return None;
                }
            };
            scanner.set_pos(end + 1);
            Some(AttrValue {
                parts: vec![AttrValuePart::Expression {
                    expression_range: Range::new(expr_start, end),
                    range: Range::new(start, end + 1),
                }],
                range: Range::new(start, end + 1),
                quoted: false,
            })
        }
        _ => {
            // Unquoted literal value — read until whitespace/>/.
            let start = scanner.pos();
            while let Some(b) = scanner.peek_byte() {
                if b.is_ascii_whitespace() || matches!(b, b'>' | b'/') {
                    break;
                }
                scanner.advance_char();
            }
            let end = scanner.pos();
            let content = scanner.source()[start as usize..end as usize].to_string();
            Some(AttrValue {
                parts: vec![AttrValuePart::Text {
                    content,
                    range: Range::new(start, end),
                }],
                range: Range::new(start, end),
                quoted: false,
            })
        }
    }
}

fn parse_quoted_value(
    scanner: &mut Scanner<'_>,
    _errors: &mut Vec<ParseError>,
) -> Option<AttrValue> {
    let start = scanner.pos();
    let quote = scanner.peek_byte()?;
    scanner.advance_byte();

    let mut parts: Vec<AttrValuePart> = Vec::new();
    let mut chunk_start = scanner.pos();

    while let Some(b) = scanner.peek_byte() {
        if b == quote {
            let chunk_end = scanner.pos();
            if chunk_end > chunk_start {
                let content =
                    scanner.source()[chunk_start as usize..chunk_end as usize].to_string();
                parts.push(AttrValuePart::Text {
                    content,
                    range: Range::new(chunk_start, chunk_end),
                });
            }
            scanner.advance_byte();
            return Some(AttrValue {
                parts,
                range: Range::new(start, scanner.pos()),
                quoted: true,
            });
        }
        if b == b'{' {
            // Interpolation `{...}` inside the quoted value (Svelte
            // attribute concatenation: `class="foo {bar} baz"`).
            // Flush the preceding text, parse the mustache body using
            // the shared brace-balancing scanner, and push an
            // Expression part covering the body.
            let text_end = scanner.pos();
            if text_end > chunk_start {
                let content = scanner.source()[chunk_start as usize..text_end as usize].to_string();
                parts.push(AttrValuePart::Text {
                    content,
                    range: Range::new(chunk_start, text_end),
                });
            }
            let brace_start = scanner.pos();
            scanner.advance_byte(); // past `{`
            let expr_start = scanner.pos();
            let Some(end) = find_mustache_end(scanner.source(), expr_start) else {
                // Unterminated mustache inside the quoted attribute —
                // best-effort: stop here and return what we have. The
                // caller will continue scanning the surrounding markup.
                return Some(AttrValue {
                    parts,
                    range: Range::new(start, scanner.pos()),
                    quoted: true,
                });
            };
            scanner.set_pos(end + 1);
            parts.push(AttrValuePart::Expression {
                expression_range: Range::new(expr_start, end),
                range: Range::new(brace_start, end + 1),
            });
            chunk_start = scanner.pos();
            continue;
        }
        scanner.advance_char();
    }

    // Unterminated string — consume to EOF and return what we have.
    let text_end = scanner.pos();
    if text_end > chunk_start {
        let content = scanner.source()[chunk_start as usize..text_end as usize].to_string();
        parts.push(AttrValuePart::Text {
            content,
            range: Range::new(chunk_start, text_end),
        });
    }
    Some(AttrValue {
        parts,
        range: Range::new(start, scanner.pos()),
        quoted: true,
    })
}

/// Find a top-level comma inside a mustache expression range.
///
/// Used to detect `bind:value={getter, setter}` pairs — a SequenceExpression
/// where the top-level comma separates two arrow functions.
///
/// The scan respects string literals, template literals, line/block
/// comments, and nested parens/brackets/braces. Returns the byte offset of
/// the comma, or `None` if none is found at depth 0.
fn find_top_level_comma(src: &str, start: u32, end: u32) -> Option<u32> {
    let bytes = src.as_bytes();
    let mut i = start as usize;
    let end = end as usize;
    let mut depth: i32 = 0;
    // Re-use the same template-literal-stack idiom as `find_mustache_end`.
    let mut template_stack: Vec<i32> = Vec::new();

    while i < end {
        let b = bytes[i];
        match b {
            b',' if depth == 0 && template_stack.is_empty() => return Some(i as u32),
            b'(' | b'[' | b'{' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' | b'}' => {
                depth -= 1;
                i += 1;
            }
            b'"' => {
                i = skip_string(bytes, i + 1, b'"', end)?;
            }
            b'\'' => {
                i = skip_string(bytes, i + 1, b'\'', end)?;
            }
            b'`' => {
                i = skip_template_literal(bytes, i + 1, end, &mut template_stack, &mut depth)?;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                // Line comment to newline or end.
                i += 2;
                while i < end && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < end {
                    if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    None
}

fn skip_string(bytes: &[u8], start: usize, quote: u8, limit: usize) -> Option<usize> {
    let mut i = start;
    while i < limit {
        match bytes[i] {
            b if b == quote => return Some(i + 1),
            b'\\' => i += 2,
            b'\n' => return Some(i + 1),
            _ => i += 1,
        }
    }
    Some(limit)
}

fn skip_template_literal(
    bytes: &[u8],
    start: usize,
    limit: usize,
    template_stack: &mut Vec<i32>,
    outer_depth: &mut i32,
) -> Option<usize> {
    let mut i = start;
    while i < limit {
        match bytes[i] {
            b'`' => return Some(i + 1),
            b'\\' => i += 2,
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                template_stack.push(*outer_depth);
                *outer_depth += 1;
                return Some(i + 2);
            }
            _ => i += 1,
        }
    }
    Some(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> (Vec<Attribute>, Vec<ParseError>) {
        let mut scanner = Scanner::new(src);
        let mut errors = Vec::new();
        let attrs = parse_attributes(&mut scanner, src.len() as u32, &mut errors);
        (attrs, errors)
    }

    fn parse_ok(src: &str) -> Vec<Attribute> {
        let (attrs, errors) = parse(src);
        assert!(errors.is_empty(), "expected no errors, got {errors:?}");
        attrs
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(parse_ok("").is_empty());
    }

    #[test]
    fn stops_at_closing_gt() {
        let (attrs, _) = parse(">");
        assert!(attrs.is_empty());
    }

    #[test]
    fn plain_string_attr_double_quoted() {
        let attrs = parse_ok(r#"class="foo""#);
        assert_eq!(attrs.len(), 1);
        let Attribute::Plain(a) = &attrs[0] else {
            panic!("expected Plain");
        };
        assert_eq!(a.name, "class");
        let v = a.value.as_ref().unwrap();
        assert!(v.quoted);
        assert_eq!(v.parts.len(), 1);
        if let AttrValuePart::Text { content, .. } = &v.parts[0] {
            assert_eq!(content, "foo");
        } else {
            panic!("expected Text");
        }
    }

    #[test]
    fn plain_string_attr_single_quoted() {
        let attrs = parse_ok(r#"id='x'"#);
        let Attribute::Plain(a) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(a.name, "id");
    }

    #[test]
    fn plain_unquoted_value() {
        let attrs = parse_ok("tabindex=0");
        let Attribute::Plain(a) = &attrs[0] else {
            unreachable!()
        };
        let v = a.value.as_ref().unwrap();
        assert!(!v.quoted);
    }

    #[test]
    fn boolean_attr_no_equals() {
        let attrs = parse_ok("disabled");
        let Attribute::Plain(a) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(a.name, "disabled");
        assert!(a.value.is_none());
    }

    #[test]
    fn multiple_attrs() {
        let attrs = parse_ok(r#"class="foo" id="bar" disabled"#);
        assert_eq!(attrs.len(), 3);
    }

    #[test]
    fn expression_attr() {
        let attrs = parse_ok("onclick={handler}");
        let Attribute::Expression(a) = &attrs[0] else {
            panic!("expected Expression, got {:?}", attrs[0]);
        };
        assert_eq!(a.name, "onclick");
        assert_eq!(a.expression_range.slice("onclick={handler}"), "handler");
    }

    #[test]
    fn shorthand_attr() {
        let attrs = parse_ok("{href}");
        let Attribute::Shorthand(a) = &attrs[0] else {
            panic!("expected Shorthand");
        };
        assert_eq!(a.name, "href");
    }

    #[test]
    fn spread_attr() {
        let attrs = parse_ok("{...attrs}");
        let Attribute::Spread(a) = &attrs[0] else {
            panic!("expected Spread");
        };
        assert_eq!(a.expression_range.slice("{...attrs}"), "attrs");
    }

    #[test]
    fn bind_directive_simple() {
        let attrs = parse_ok("bind:value={x}");
        let Attribute::Directive(d) = &attrs[0] else {
            panic!("expected Directive");
        };
        assert_eq!(d.kind, DirectiveKind::Bind);
        assert_eq!(d.name, "value");
        let Some(DirectiveValue::Expression {
            expression_range, ..
        }) = &d.value
        else {
            panic!("expected Expression value");
        };
        assert_eq!(expression_range.slice("bind:value={x}"), "x");
    }

    #[test]
    fn bind_directive_shorthand() {
        // `bind:value` with no `=` is the Svelte shorthand for `bind:value={value}`.
        let attrs = parse_ok("bind:value");
        let Attribute::Directive(d) = &attrs[0] else {
            unreachable!()
        };
        assert!(d.value.is_none());
    }

    #[test]
    fn bind_directive_with_getter_setter_pair() {
        let attrs = parse_ok("bind:value={() => g(), (v) => s(v)}");
        let Attribute::Directive(d) = &attrs[0] else {
            panic!("expected Directive");
        };
        let Some(DirectiveValue::BindPair {
            getter_range,
            setter_range,
            ..
        }) = &d.value
        else {
            panic!("expected BindPair, got {:?}", d.value);
        };
        assert_eq!(
            getter_range
                .slice("bind:value={() => g(), (v) => s(v)}")
                .trim(),
            "() => g()"
        );
        assert_eq!(
            setter_range
                .slice("bind:value={() => g(), (v) => s(v)}")
                .trim(),
            "(v) => s(v)"
        );
    }

    #[test]
    fn bind_expression_with_inner_comma_is_not_a_pair() {
        // Comma inside a function call is NOT a top-level comma.
        let attrs = parse_ok("bind:value={foo(a, b)}");
        let Attribute::Directive(d) = &attrs[0] else {
            unreachable!()
        };
        assert!(matches!(d.value, Some(DirectiveValue::Expression { .. })));
    }

    #[test]
    fn on_directive_with_modifiers() {
        let attrs = parse_ok("on:click|once|preventDefault={handler}");
        let Attribute::Directive(d) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(d.kind, DirectiveKind::On);
        assert_eq!(d.name, "click");
        assert_eq!(
            d.modifiers,
            vec![SmolStr::from("once"), SmolStr::from("preventDefault")]
        );
    }

    #[test]
    fn use_directive_with_no_value() {
        let attrs = parse_ok("use:tooltip");
        let Attribute::Directive(d) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(d.kind, DirectiveKind::Use);
        assert!(d.value.is_none());
    }

    #[test]
    fn class_directive_with_expression() {
        let attrs = parse_ok("class:active={isActive}");
        let Attribute::Directive(d) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(d.kind, DirectiveKind::Class);
        assert_eq!(d.name, "active");
    }

    #[test]
    fn style_directive_with_quoted_literal() {
        let attrs = parse_ok(r#"style:left="100px""#);
        let Attribute::Directive(d) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(d.kind, DirectiveKind::Style);
        assert_eq!(d.name, "left");
        let Some(DirectiveValue::Quoted(av)) = &d.value else {
            panic!("expected Quoted");
        };
        assert!(av.quoted);
    }

    #[test]
    fn transition_directive_with_local_modifier() {
        let attrs = parse_ok("transition:fade|local={{ duration: 200 }}");
        let Attribute::Directive(d) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(d.kind, DirectiveKind::Transition);
        assert_eq!(d.name, "fade");
        assert_eq!(d.modifiers, vec![SmolStr::from("local")]);
    }

    #[test]
    fn in_and_out_directives() {
        let attrs = parse_ok("in:fly out:slide");
        assert_eq!(attrs.len(), 2);
        let Attribute::Directive(a) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(a.kind, DirectiveKind::In);
        let Attribute::Directive(b) = &attrs[1] else {
            unreachable!()
        };
        assert_eq!(b.kind, DirectiveKind::Out);
    }

    #[test]
    fn animate_directive() {
        let attrs = parse_ok("animate:flip");
        let Attribute::Directive(d) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(d.kind, DirectiveKind::Animate);
    }

    #[test]
    fn let_directive() {
        let attrs = parse_ok("let:item");
        let Attribute::Directive(d) = &attrs[0] else {
            unreachable!()
        };
        assert_eq!(d.kind, DirectiveKind::Let);
        assert_eq!(d.name, "item");
    }

    #[test]
    fn mixed_attrs_and_directives() {
        let attrs = parse_ok(r#"class="foo" {...rest} on:click={handler} bind:value"#);
        assert_eq!(attrs.len(), 4);
        assert!(matches!(attrs[0], Attribute::Plain(_)));
        assert!(matches!(attrs[1], Attribute::Spread(_)));
        assert!(matches!(attrs[2], Attribute::Directive(_)));
        assert!(matches!(attrs[3], Attribute::Directive(_)));
    }

    #[test]
    fn directive_kind_round_trip() {
        for kind in [
            DirectiveKind::Bind,
            DirectiveKind::On,
            DirectiveKind::Use,
            DirectiveKind::Class,
            DirectiveKind::Style,
            DirectiveKind::Transition,
            DirectiveKind::In,
            DirectiveKind::Out,
            DirectiveKind::Animate,
            DirectiveKind::Let,
        ] {
            assert_eq!(DirectiveKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(DirectiveKind::parse("nope"), None);
    }

    #[test]
    fn find_top_level_comma_basic() {
        // "a, b" — comma at offset 1.
        let src = "_{a, b}_";
        // We're passing just the expression body range.
        assert_eq!(find_top_level_comma(src, 2, 6), Some(3));
    }

    #[test]
    fn find_top_level_comma_ignores_nested() {
        let src = "_{foo(a, b), bar}_";
        // Only the comma after `)` is top-level.
        let top = find_top_level_comma(src, 2, 16).unwrap();
        assert_eq!(&src[top as usize..=top as usize], ",");
        assert_eq!(top, 11);
    }

    #[test]
    fn find_top_level_comma_ignores_in_string() {
        let src = r#"_{"a,b", 1}_"#;
        let top = find_top_level_comma(src, 2, 10).unwrap();
        // Only the comma between the string and `1`.
        assert_eq!(top, 7);
    }
}
