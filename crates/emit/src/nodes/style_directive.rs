//! `style:prop={value}` / `style:prop="…{expr}…"` / `style:prop`
//! (shorthand) directive emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/StyleDirective.ts`.
//!
//! Each style directive's value is wrapped in
//! `__svn_ensure_type(String, Number, value)` so it type-checks against
//! the CSS-value union (`String | Number | null | undefined`). Fires
//! TS2345 on non-stringish values and TS18046 / TS7005 on reads of
//! `unknown` / implicit-any bindings inside the value expression.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// Emit a `style:prop={EXPR}` / `style:prop="…{EXPR}…"` directive as a
/// bare `__svn_ensure_type(String, Number, …)` call.
pub(crate) fn emit_style_directive(
    buf: &mut EmitBuffer,
    source: &str,
    d: &svn_parser::Directive,
    indent: &str,
) {
    match &d.value {
        Some(svn_parser::DirectiveValue::Expression {
            expression_range, ..
        }) => {
            let Some(expr) =
                source.get(expression_range.start as usize..expression_range.end as usize)
            else {
                return;
            };
            let trimmed = expr.trim();
            if trimmed.is_empty() {
                return;
            }
            let leading_ws = (expr.len() - expr.trim_start().len()) as u32;
            let start = expression_range.start + leading_ws;
            let end = start + trimmed.len() as u32;
            let _ = write!(buf, "{indent}__svn_ensure_type(String, Number, ");
            buf.append_with_source(trimmed, svn_core::Range::new(start, end));
            let _ = writeln!(buf, ");");
        }
        Some(svn_parser::DirectiveValue::Quoted(av)) => {
            emit_template_literal(buf, source, av, indent);
        }
        _ => {}
    }
}

/// Emit the template-literal form for a quoted
/// `style:prop="…{expr}…"` AttrValue. Produces
/// `__svn_ensure_type(String, Number, <template-literal>);` where the
/// template literal splices in each `Text` part verbatim and each
/// `Expression` part as a `${…}` interpolation. Each expression
/// interpolation carries a TokenMapEntry covering the user-source
/// range so tsgo diagnostics fired inside map back to the correct
/// source position.
fn emit_template_literal(
    buf: &mut EmitBuffer,
    source: &str,
    av: &svn_parser::AttrValue,
    indent: &str,
) {
    if av.parts.is_empty() {
        return;
    }
    let _ = write!(buf, "{indent}__svn_ensure_type(String, Number, `");
    for part in &av.parts {
        match part {
            svn_parser::AttrValuePart::Text { content, .. } => {
                for ch in content.chars() {
                    match ch {
                        '`' => buf.push_str("\\`"),
                        '\\' => buf.push_str("\\\\"),
                        '$' => buf.push_str("\\$"),
                        _ => {
                            let mut b = [0u8; 4];
                            buf.push_str(ch.encode_utf8(&mut b));
                        }
                    }
                }
            }
            svn_parser::AttrValuePart::Expression {
                expression_range, ..
            } => {
                let Some(expr) =
                    source.get(expression_range.start as usize..expression_range.end as usize)
                else {
                    continue;
                };
                let trimmed = expr.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let leading_ws = (expr.len() - expr.trim_start().len()) as u32;
                let start = expression_range.start + leading_ws;
                let end = start + trimmed.len() as u32;
                buf.push_str("${");
                buf.append_with_source(trimmed, svn_core::Range::new(start, end));
                buf.push_str("}");
            }
        }
    }
    let _ = writeln!(buf, "`);");
}
