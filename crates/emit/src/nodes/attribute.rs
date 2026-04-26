//! Plain / expression / shorthand attribute emission for DOM elements.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Attribute.ts`.
//!
//! Each handler emits one entry of the `svelteHTML.createElement("tag", {
//! …entries })` literal that [`crate::nodes::element::emit_dom_element_open`]
//! is currently building. The attribute-skip table (`should_skip`) is
//! also here so the dispatcher consults a single source of truth.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// Drop attributes that the svelte-jsx typings reject at the strict
/// interface but real Svelte allows: `data-*`, `aria-*`, CSS custom
/// properties, namespaced (`xml:lang`, `xlink:href`), the `this`
/// directive on `<svelte:element>`, the `slot=""` directive, and
/// React-style camelCase synonyms whose lowercase counterparts svelte-jsx
/// declares.
pub(crate) fn should_skip(name: &str) -> bool {
    if name.starts_with("data-") || name.starts_with("aria-") {
        return true;
    }
    if name.starts_with("--") {
        return true;
    }
    if name.contains(':') {
        return true;
    }
    if name == "this" {
        return true;
    }
    if name == "slot" {
        return true;
    }
    if is_react_camelcase_synonym(name) {
        return true;
    }
    false
}

/// React-style camelCase HTML attribute names that have a lowercase
/// HTML5 counterpart. svelte-jsx declares only the lowercase form;
/// upstream silently drops these via source-map filtering.
fn is_react_camelcase_synonym(name: &str) -> bool {
    matches!(
        name,
        "tabIndex"
            | "className"
            | "htmlFor"
            | "readOnly"
            | "maxLength"
            | "minLength"
            | "colSpan"
            | "rowSpan"
            | "autoFocus"
            | "autoComplete"
            | "autoCorrect"
            | "autoCapitalize"
            | "spellCheck"
            | "contentEditable"
    )
}

/// Attributes that svelte-elements types as `number | undefined | null`.
/// Upstream svelte2tsx's `Attribute.ts::numberOnlyAttributes` — when
/// the attribute value is a pure-numeric Text (no `{expr}` interpolation),
/// emit the value as a bare number literal instead of a string template
/// so TS binds it against the typed number slot.
fn is_number_only_attr(name: &str) -> bool {
    matches!(
        name,
        "aria-colcount"
            | "aria-colindex"
            | "aria-colspan"
            | "aria-level"
            | "aria-posinset"
            | "aria-rowcount"
            | "aria-rowindex"
            | "aria-rowspan"
            | "aria-setsize"
            | "aria-valuemax"
            | "aria-valuemin"
            | "aria-valuenow"
            | "results"
            | "span"
            | "marginheight"
            | "marginwidth"
            | "maxlength"
            | "minlength"
            | "currenttime"
            | "defaultplaybackrate"
            | "volume"
            | "high"
            | "low"
            | "optimum"
            | "start"
            | "size"
            | "border"
            | "cols"
            | "rows"
            | "colspan"
            | "rowspan"
            | "tabindex"
    )
}

pub(crate) fn emit_plain(
    buf: &mut EmitBuffer,
    source: &str,
    p: &svn_parser::PlainAttr,
    depth: usize,
) {
    let indent = "    ".repeat(depth);
    let name = p.name.as_str();
    let name_range = svn_core::Range::new(p.range.start, p.range.start + name.len() as u32);
    let key_text = format!("\"{name}\"");
    match &p.value {
        None => {
            // Boolean attribute: `<input required>` → `"required": true,`
            // `<div popover>` carve-out: upstream emits `"": ""`.
            let value = if name == "popover" { "\"\"" } else { "true" };
            buf.push_str(&indent);
            buf.append_with_source(&key_text, name_range);
            let _ = writeln!(buf, ": {value},");
        }
        Some(v) if v.parts.is_empty() => {
            buf.push_str(&indent);
            buf.append_with_source(&key_text, name_range);
            buf.push_str(": \"\",\n");
        }
        Some(v) if v.parts.len() == 1 => {
            match &v.parts[0] {
                svn_parser::AttrValuePart::Text { content, .. } => {
                    // numberOnlyAttributes carve-out: emit bare number
                    // literal when the text parses as a number.
                    if is_number_only_attr(name)
                        && !content.is_empty()
                        && content.trim().parse::<f64>().is_ok()
                    {
                        buf.push_str(&indent);
                        buf.append_with_source(&key_text, name_range);
                        let _ = writeln!(buf, ": {},", content.trim());
                        return;
                    }
                    let escaped = content.replace('\\', "\\\\").replace('`', "\\`");
                    buf.push_str(&indent);
                    buf.append_with_source(&key_text, name_range);
                    let _ = writeln!(buf, ": `{escaped}`,");
                }
                svn_parser::AttrValuePart::Expression {
                    expression_range, ..
                } => {
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
                    buf.push_str(&indent);
                    buf.append_with_source(&key_text, name_range);
                    buf.push_str(": (");
                    buf.append_with_source(trimmed, svn_core::Range::new(start, end));
                    let _ = writeln!(buf, "),");
                }
            }
        }
        Some(v) => {
            // Multi-part (text + interpolations). Template literal
            // with `${expr}` placeholders so the whole attribute
            // binds as a string. Follows upstream Attribute.ts's
            // multi-value branch.
            buf.push_str(&indent);
            buf.append_with_source(&key_text, name_range);
            buf.push_str(": `");
            for part in &v.parts {
                match part {
                    svn_parser::AttrValuePart::Text { content, .. } => {
                        let escaped = content.replace('\\', "\\\\").replace('`', "\\`");
                        buf.push_str(&escaped);
                    }
                    svn_parser::AttrValuePart::Expression {
                        expression_range, ..
                    } => {
                        let Some(expr) = source
                            .get(expression_range.start as usize..expression_range.end as usize)
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
            let _ = writeln!(buf, "`,");
        }
    }
}

pub(crate) fn emit_expression(
    buf: &mut EmitBuffer,
    source: &str,
    e: &svn_parser::ExpressionAttr,
    depth: usize,
) {
    let indent = "    ".repeat(depth);
    let Some(expr) = source.get(e.expression_range.start as usize..e.expression_range.end as usize)
    else {
        return;
    };
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return;
    }
    let leading_ws = (expr.len() - expr.trim_start().len()) as u32;
    let start = e.expression_range.start + leading_ws;
    let end = start + trimmed.len() as u32;
    let name = e.name.as_str();
    buf.push_str(&indent);
    let name_range = svn_core::Range::new(e.range.start, e.range.start + name.len() as u32);
    buf.append_with_source(&format!("\"{name}\""), name_range);
    buf.push_str(": (");
    buf.append_with_source(trimmed, svn_core::Range::new(start, end));
    let _ = writeln!(buf, "),");
}

pub(crate) fn emit_shorthand(
    buf: &mut EmitBuffer,
    source: &str,
    s: &svn_parser::ShorthandAttr,
    depth: usize,
) {
    let indent = "    ".repeat(depth);
    let name = s.name.as_str();
    let inner = source
        .get(s.range.start as usize + 1..s.range.end as usize)
        .unwrap_or("");
    let leading_ws = (inner.len() - inner.trim_start().len()) as u32;
    let name_start = s.range.start + 1 + leading_ws;
    let name_end = name_start + name.len() as u32;
    buf.push_str(&indent);
    buf.append_with_source(name, svn_core::Range::new(name_start, name_end));
    buf.push_str(",\n");
}
