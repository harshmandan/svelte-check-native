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
//! ### Attribute-value interpolations
//!
//! Both quoted (`class="a {b} c"`) and unquoted (`label=hi{x}there`)
//! attribute values flush Text chunks on `{` and parse the mustache
//! body via the shared brace-balancing scanner, producing mixed
//! Text / Expression `AttrValuePart`s. Downstream emit (component
//! prop literals, element createElement attrs) walks the parts and
//! reassembles each value as a TS template literal, so embedded
//! expressions get contextual typing and identifier references are
//! visible to TS.

use smol_str::SmolStr;
use svn_core::Range;

use crate::ast::{
    AttrComment, AttrValue, AttrValuePart, Attribute, Directive, DirectiveKind, DirectiveValue,
    ExpressionAttr, PlainAttr, ShorthandAttr, SpreadAttr,
};
use crate::error::ParseError;
use crate::mustache::find_mustache_end;
use crate::scanner::{Scanner, is_svelte_whitespace_at, unicode_identifier_len};

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
        // Attributes separate on the compiler's full whitespace class —
        // an accidental NBSP between attributes is still a separator.
        scanner.skip_svelte_whitespace();

        if scanner.pos() >= fragment_end || scanner.eof() {
            // Unterminated tag; let the caller report the higher-level error.
            return attrs;
        }

        match scanner.peek_byte() {
            Some(b'>') | None => return attrs,
            // `/` is the start of `/>` (self-close) UNLESS it begins an
            // in-tag JS comment (`//` or `/* */`, a Svelte 5 feature).
            Some(b'/') => match scanner.peek_byte_at(1) {
                Some(b'/') | Some(b'*') => {
                    attrs.push(Attribute::Comment(parse_intag_comment(
                        scanner,
                        fragment_end,
                    )));
                }
                _ => return attrs,
            },
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

/// Consume an in-tag JS comment (`//…` to end of line, or `/*…*/`),
/// bounded by `fragment_end`. The scanner is positioned at the leading
/// `/`.
fn parse_intag_comment(scanner: &mut Scanner<'_>, fragment_end: u32) -> AttrComment {
    let start = scanner.pos();
    let block = scanner.peek_byte_at(1) == Some(b'*');
    scanner.advance_byte(); // `/`
    scanner.advance_byte(); // `/` or `*`
    if block {
        while scanner.pos() < fragment_end && !scanner.eof() {
            if scanner.peek_byte() == Some(b'*') && scanner.peek_byte_at(1) == Some(b'/') {
                scanner.advance_byte();
                scanner.advance_byte();
                break;
            }
            scanner.advance_byte();
        }
    } else {
        // Line comment runs to the newline (not consumed — the loop's
        // whitespace skip handles it).
        while scanner.pos() < fragment_end && !scanner.eof() && scanner.peek_byte() != Some(b'\n') {
            scanner.advance_byte();
        }
    }
    AttrComment {
        range: Range::new(start, scanner.pos()),
        block,
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
    // after whitespace). Identifiers are full-Unicode (upstream
    // read_identifier uses acorn's isIdentifierStart/Char), so `{変数}`
    // is a valid shorthand.
    let name_start = cursor.pos();
    let name_end = {
        let src = scanner.source();
        let rest = &src[name_start as usize..];
        name_start + unicode_identifier_len(rest)
    };
    cursor.set_pos(name_end);

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

    // Read the attribute name. Directive names include `:`; HTML attrs
    // can include hyphens, digits, underscores, and (rarely) `$`. `|`
    // is part of directive-modifier syntax (`on:click|once`) — valid
    // only after a `:` but harmless to allow generally since plain
    // HTML attrs don't use pipes.
    //
    // The upstream Svelte compiler reads attribute and directive names
    // by terminating only on `\s`, `=`, `/`, `>`, `"`, `'` — every other
    // byte is part of the name. Mirror that here so Tailwind/UnoCSS
    // class-directive shapes round-trip:
    //   `class:!font-semibold={cond}`            — `!` important prefix
    //   `class:*:bg-red-500={cond}`              — `*:` child variant
    //   `class:[aspect-16/9]={cond}`             — `/` only inside `[…]`
    //   `class:grid-cols-[1fr_500px]={cond}`     — bracketed value
    //   `class:bg-[var(--my-prop)]={cond}`       — parens inside brackets
    //   `class:grid-cols-[minmax(0,1fr)]={cond}` — commas inside brackets
    // Terminator-set enforcement (instead of an alphanumeric allowlist)
    // is what makes the parser robust to Tailwind's evolving sigil
    // grammar without a code change per character — pre-fix, an
    // unsupported byte truncated the name, the rest fell through to
    // `parse_named_attribute` recovery as a separate plain attribute,
    // and downstream emit reported it as an unknown DOM prop (TS2353
    // — see gh#14). Bracket depth is still tracked so a closing `]`
    // doesn't mistakenly terminate when the depth-0 terminator set
    // (which excludes `]`) is in effect inside `[...]` — e.g. so
    // `class:[&>li]:foo` doesn't end at the inner `>`.
    //
    // `<` is the only depth-0 terminator we keep that upstream's
    // regex doesn't include — Svelte's parser allows `<` inside
    // attribute names but practical templates don't, and stopping
    // there speeds up malformed-tag recovery in the surrounding
    // template walker.
    //
    // `/` keeps terminating at depth 0 — without lookahead, accepting
    // it would swallow the self-closing `/>` terminator (`<input
    // bind:value/>`). Inside `[...]` upstream allows it, and so do
    // we; that's where Tailwind's `aspect-16/9` shape lives.
    let name_start = scanner.pos();
    let mut bracket_depth: u32 = 0;
    while let Some(b) = scanner.peek_byte() {
        // Upstream terminates names on `\s` — the full JS whitespace
        // class — so a Unicode space (NBSP etc.) between attributes
        // ends the name like an ASCII space does. Other non-ASCII
        // chars are part of the name.
        if b >= 0x80 {
            if is_svelte_whitespace_at(scanner.source().as_bytes(), scanner.pos() as usize) {
                break;
            }
            scanner.advance_char();
            continue;
        }
        if bracket_depth > 0 {
            if matches!(
                b,
                b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'>' | b'<' | b'"' | b'\''
            ) {
                break;
            }
            if b == b'[' {
                bracket_depth += 1;
            } else if b == b']' {
                bracket_depth -= 1;
            }
            scanner.advance_byte();
        } else if matches!(
            b,
            b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'/' | b'>' | b'<' | b'"' | b'\''
        ) {
            break;
        } else if b == b'[' {
            bracket_depth += 1;
            scanner.advance_byte();
        } else {
            scanner.advance_byte();
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

    // Plain-style named attribute. Svelte's `read_attribute` allows
    // whitespace around `=` (`allow_whitespace()` on both sides), so
    // `class = "foo"` is a single valued attribute, not a boolean
    // `class` plus a stray `foo`. Peek past whitespace to decide
    // whether a value follows; only commit when we actually see `=`.
    let name_sym: SmolStr = name.into();
    let after_name = scanner.pos();
    scanner.skip_ascii_whitespace();
    let value = if scanner.peek_byte() == Some(b'=') {
        scanner.advance_byte();
        scanner.skip_ascii_whitespace();
        match parse_attr_value(scanner, errors) {
            Some(v) => Some(v),
            None => return None,
        }
    } else {
        // Boolean attribute — restore the scanner to just after the
        // name so the attribute-list loop sees the trailing whitespace
        // exactly as before.
        scanner.set_pos(after_name);
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

    let after_name = scanner.pos();
    scanner.skip_ascii_whitespace();
    let value = if scanner.peek_byte() == Some(b'=') {
        scanner.advance_byte();
        scanner.skip_ascii_whitespace();
        Some(parse_directive_value(scanner, kind, errors)?)
    } else {
        // Boolean directive — restore so the attr loop sees trailing
        // whitespace exactly as before and the range doesn't extend over it.
        scanner.set_pos(after_name);
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

            // For bind:foo={getter, setter}, detect the pair comma.
            // Textual comma scanning can't decide this: the depth
            // tracker doesn't nest `<...>`, so the type-argument comma
            // in `bind:value={fn<A, B>()}` looks top-level too.
            // Upstream acorn-parses the whole value and a pair is
            // exactly a two-element SequenceExpression — mirror that
            // with an oxc probe parse.
            if matches!(kind, DirectiveKind::Bind)
                && let Some(comma_pos) = bind_pair_split(scanner.source(), expr_start, end)
            {
                return Some(DirectiveValue::BindPair {
                    getter_range: Range::new(expr_start, comma_pos),
                    setter_range: Range::new(comma_pos + 1, end),
                    range: Range::new(start, end + 1),
                });
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
                if b.is_ascii_whitespace() || matches!(b, b'>' | b'"' | b'\'' | b'=' | b'<' | b'`')
                {
                    break;
                }
                // A bare `/` is part of an unquoted value; only `/>` ends it.
                if b == b'/' && scanner.peek_byte_at(1) == Some(b'>') {
                    break;
                }
                scanner.advance_char();
            }
            let end = scanner.pos();
            Some(DirectiveValue::Quoted(AttrValue {
                parts: vec![AttrValuePart::Text {
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
            // Unquoted literal value — read until whitespace/>/, but
            // also recognize `{…}` interpolations so the user-visible
            // shape of `foo=hi{bar}hi` mirrors the quoted form
            // (`foo="hi{bar}hi"`). Reviewer follow-up #4: pre-fix
            // this parser produced ONE Text part with the literal
            // string content `hi{bar}hi` — the `{bar}` interpolation
            // was never extracted as an expression and downstream
            // emit silently typed it as a literal substring.
            //
            // Mirrors the quoted-value parser at `parse_quoted_value`
            // above: flush a Text chunk on `{`, scan the mustache
            // body via the shared brace-balancing scanner, push an
            // Expression part. Terminator remains whitespace / `>`
            // / `/`.
            let start = scanner.pos();
            let mut parts: Vec<AttrValuePart> = Vec::new();
            let mut chunk_start = start;
            while let Some(b) = scanner.peek_byte() {
                if b.is_ascii_whitespace() || matches!(b, b'>' | b'"' | b'\'' | b'=' | b'<' | b'`')
                {
                    break;
                }
                // A bare `/` is part of an unquoted value (`href=/foo`,
                // `src=//cdn/x.js`). Svelte terminates unquoted values
                // only at whitespace or `>`; the sole `/` exception is
                // the self-closing `/>`. So break on `/` ONLY when it's
                // immediately followed by `>`.
                if b == b'/' && scanner.peek_byte_at(1) == Some(b'>') {
                    break;
                }
                if b == b'{' {
                    let text_end = scanner.pos();
                    if text_end > chunk_start {
                        parts.push(AttrValuePart::Text {
                            range: Range::new(chunk_start, text_end),
                        });
                    }
                    let brace_start = scanner.pos();
                    scanner.advance_byte(); // past `{`
                    let expr_start = scanner.pos();
                    let Some(end) = find_mustache_end(scanner.source(), expr_start) else {
                        // Unterminated mustache — best-effort: emit
                        // what we have so far, terminate the value.
                        let trailing_end = scanner.pos();
                        return Some(AttrValue {
                            parts,
                            range: Range::new(start, trailing_end),
                            quoted: false,
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
            let end = scanner.pos();
            if end > chunk_start {
                parts.push(AttrValuePart::Text {
                    range: Range::new(chunk_start, end),
                });
            }
            // Empty-value edge: scanner immediately hit a terminator.
            // Preserve a single empty Text part so downstream
            // `parts.len() == 1` literal-value handling still
            // matches.
            if parts.is_empty() {
                parts.push(AttrValuePart::Text {
                    range: Range::new(start, end),
                });
            }
            Some(AttrValue {
                parts,
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
                parts.push(AttrValuePart::Text {
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
                parts.push(AttrValuePart::Text {
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
        // Neither the closing quote nor `{` (both handled above) —
        // hop to the next occurrence of either in one memchr2 sweep.
        // Long literal values (Tailwind class strings) previously
        // paid a peek+advance per byte here.
        scanner.skip_until2(quote, b'{');
    }

    // Unterminated string — consume to EOF and return what we have.
    let text_end = scanner.pos();
    if text_end > chunk_start {
        parts.push(AttrValuePart::Text {
            range: Range::new(chunk_start, text_end),
        });
    }
    Some(AttrValue {
        parts,
        range: Range::new(start, scanner.pos()),
        quoted: true,
    })
}

/// Byte offset of the comma splitting a `bind:x={get, set}` function
/// pair, or `None` when the value is a single expression.
///
/// Decided the way upstream does: parse the whole value (acorn there,
/// oxc here, TS mode) — a pair is exactly a two-element
/// SequenceExpression. A TS type-argument comma (`fn<A, B>()`) parses
/// as a single CallExpression and never splits; a generic call inside a
/// pair half (`() => pick<A, B>(x), (v) => sink(v)`) still yields the
/// two-arrow sequence with the split at the real pair comma.
fn bind_pair_split(src: &str, start: u32, end: u32) -> Option<u32> {
    use oxc_ast::ast::{Expression, Statement};

    let text = &src[start as usize..end as usize];
    if !text.contains(',') {
        return None;
    }
    let allocator = oxc_allocator::Allocator::default();
    let probe = format!("({text});");
    let parsed = oxc_parser::Parser::new(
        &allocator,
        &probe,
        oxc_span::SourceType::default().with_typescript(true),
    )
    .parse();
    if !parsed.diagnostics.is_empty() || parsed.panicked || parsed.program.body.len() != 1 {
        return None;
    }
    let Statement::ExpressionStatement(stmt) = &parsed.program.body[0] else {
        return None;
    };
    let Expression::ParenthesizedExpression(paren) = &stmt.expression else {
        return None;
    };
    let Expression::SequenceExpression(seq) = &paren.expression else {
        return None;
    };
    if seq.expressions.len() != 2 {
        return None;
    }
    // The comma sits between the two expression spans. Probe offsets are
    // shifted +1 by the wrapping `(`.
    let gap_start = oxc_span::GetSpan::span(&seq.expressions[0]).end as usize;
    let gap_end = oxc_span::GetSpan::span(&seq.expressions[1]).start as usize;
    let comma_in_probe = probe[gap_start..gap_end].find(',')? + gap_start;
    Some(start + comma_in_probe as u32 - 1)
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
        let src = r#"class="foo""#;
        let attrs = parse_ok(src);
        assert_eq!(attrs.len(), 1);
        let Attribute::Plain(a) = &attrs[0] else {
            panic!("expected Plain");
        };
        assert_eq!(a.name, "class");
        let v = a.value.as_ref().unwrap();
        assert!(v.quoted);
        assert_eq!(v.parts.len(), 1);
        if let AttrValuePart::Text { range } = &v.parts[0] {
            assert_eq!(range.slice(src), "foo");
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
    fn whitespace_around_equals_is_one_valued_attr() {
        // Svelte allows `class = "foo"` (whitespace around `=`). Pre-fix
        // this parsed as boolean `class` + a stray `foo` attribute.
        let attrs = parse_ok(r#"class = "foo""#);
        assert_eq!(attrs.len(), 1, "got {attrs:?}");
        let Attribute::Plain(a) = &attrs[0] else {
            panic!("expected Plain, got {:?}", attrs[0]);
        };
        assert_eq!(a.name, "class");
        let v = a.value.as_ref().expect("class should have a value");
        assert!(v.quoted);
    }

    #[test]
    fn unquoted_value_keeps_bare_slash() {
        // `href=/foo` — a bare `/` is part of an unquoted value; only
        // `/>` terminates it. Pre-fix the `/` ended the value, leaving
        // `href` empty and `/foo` to misparse.
        let src = "href=/foo";
        let attrs = parse_ok(src);
        assert_eq!(attrs.len(), 1, "got {attrs:?}");
        let Attribute::Plain(a) = &attrs[0] else {
            panic!("expected Plain, got {:?}", attrs[0]);
        };
        assert_eq!(a.name, "href");
        let v = a.value.as_ref().expect("href should have a value");
        assert!(!v.quoted);
        // The single Text part covers the whole `/foo`.
        let AttrValuePart::Text { range } = &v.parts[0] else {
            panic!("expected Text part, got {:?}", v.parts);
        };
        assert_eq!(range.slice(src), "/foo");
    }

    #[test]
    fn unquoted_value_self_closing_still_terminates() {
        // `<input value=x/>` — the `/>` must still terminate the value
        // at `x`, not swallow the `/`.
        let attrs = parse_ok("value=x/>");
        let Attribute::Plain(a) = &attrs[0] else {
            panic!("expected Plain, got {:?}", attrs[0]);
        };
        let v = a.value.as_ref().expect("value should be present");
        let AttrValuePart::Text { range } = &v.parts[0] else {
            panic!("expected Text part");
        };
        assert_eq!(range.slice("value=x/>"), "x");
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
    fn bind_expression_with_generic_comma_is_not_a_pair() {
        // The comma in `fn<A, B>()` separates TYPE arguments — angle
        // brackets don't nest for the depth tracker, so the comma looks
        // top-level, but the halves (`fn<A` / `B>()`) aren't
        // expressions. Upstream parses the whole value with acorn-ts
        // and gets a single CallExpression.
        let src = "bind:value={fn<string, number>()}";
        let attrs = parse_ok(src);
        let Attribute::Directive(d) = &attrs[0] else {
            panic!("expected Directive");
        };
        let Some(DirectiveValue::Expression {
            expression_range, ..
        }) = &d.value
        else {
            panic!("expected single Expression, got {:?}", d.value);
        };
        assert_eq!(expression_range.slice(src), "fn<string, number>()");
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
    fn class_directive_with_bang_prefix() {
        // Tailwind v4's important modifier prefix `!` (e.g. `!font-semibold`
        // → `font-semibold !important`). Svelte's compiler accepts any
        // non-terminator byte after `class:`, so the `!` must round-trip
        // as part of the directive name. Pre-fix the scanner allowlist
        // didn't include `!`, the name truncated at `class:`, the rest
        // fell through to `parse_named_attribute` recovery, and
        // `font-semibold` ended up as a plain attribute → tsgo flagged
        // TS2353 against the element's prop type. See gh#14.
        let attrs = parse_ok("class:!font-semibold={unread}");
        assert_eq!(attrs.len(), 1, "got {:?}", attrs);
        let Attribute::Directive(d) = &attrs[0] else {
            panic!("expected directive, got {:?}", attrs[0]);
        };
        assert_eq!(d.kind, DirectiveKind::Class);
        assert_eq!(d.name, "!font-semibold");
        assert!(d.value.is_some());
    }

    #[test]
    fn class_directive_with_tailwind_names() {
        // Tailwind/UnoCSS class names cover several shapes that all need
        // to round-trip as a single directive name. Truncating at the
        // first unsupported byte would produce a malformed shorthand
        // whose emit is invalid TS and aborts tsgo's program-wide
        // type-check (one parse error → no diagnostics for any file).
        for case in [
            "mr-[1px]",
            "saturate-[.25]",
            "w-1.5",
            "[&_p]:mt-4",
            "grid-cols-[1fr_500px_2fr]",
            "grid-cols-[1fr_min-content_minmax(0,1fr)_auto]",
            "dark:hover:focus:active:group-hover:md:lg:xl:bg-blue-500",
            "bg-[var(--my-very-long-custom-property-name-from-somewhere)]",
        ] {
            let src = format!("class:{case}={{!last}}");
            let attrs = parse_ok(&src);
            let Attribute::Directive(d) = &attrs[0] else {
                panic!("expected directive for {case}, got {:?}", attrs[0]);
            };
            assert_eq!(d.kind, DirectiveKind::Class, "kind for {case}");
            assert_eq!(d.name, case, "name for {case}");
            assert!(d.value.is_some(), "value for {case}");
        }
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
    fn bind_pair_with_generic_call_in_getter_splits_at_real_comma() {
        // The type-argument comma in `pick<string, number>(x)` also
        // looks top-level (angle brackets don't nest), but its halves
        // don't parse — the split lands on the real pair comma instead.
        let src = "bind:value={() => pick<string, number>(x), (v) => sink(v)}";
        let attrs = parse_ok(src);
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
            getter_range.slice(src).trim(),
            "() => pick<string, number>(x)"
        );
        assert_eq!(setter_range.slice(src).trim(), "(v) => sink(v)");
    }
}
