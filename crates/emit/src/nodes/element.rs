//! DOM element + `<svelte:*>` element emission.
//!
//! Each `<tag …>` becomes a `svelteHTML.createElement("tag", { …attrs })`
//! call wrapped in a scoped `{ }` block. Attribute literals, expressions,
//! shorthands, and `class:` / `style:` directives are normalised here;
//! `bind:` / `use:` / `on:` directives are emitted by `directives.rs`
//! and `lib.rs::emit_element_bind_checks_inline`.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// Emit the upstream-shape `svelteHTML.createElement("tag", { …attrs });`
/// call for a DOM element. Opens a scoped `{ }` block so element-local
/// let-bindings (`{@const}`, `let:x`, action-attr `const $$action_N`)
/// stay confined to this element — matches upstream Element.ts's
/// transformation result.
///
/// Output shape (matches upstream svelte2tsx):
///   `{ svelteHTML.createElement("tag", { "name": value, name2, … }); `
/// Closing `}` is emitted by the caller after children + bind/use
/// checks recurse into the same scope. `tag_literal` controls whether
/// the first arg is a quoted string literal (`"div"`) — set false for
/// `svelte:element this={tag}` where the caller passes the expression
/// verbatim as `tag_name`.
pub(crate) fn emit_dom_element_open(
    buf: &mut EmitBuffer,
    source: &str,
    tag_name: &str,
    tag_literal: bool,
    attributes: &[svn_parser::Attribute],
    depth: usize,
    action_indices: &std::ops::Range<usize>,
) {
    let indent = "    ".repeat(depth);
    // Build the `__svn_union(__svn_action_0, __svn_action_1, …)`
    // second arg when any `use:` directives are present. Matches
    // upstream `svelte2tsx`'s 3-arg overload emit — the `attrs`
    // parameter type becomes `Elements[Key] & T` (intersection)
    // which tsgo eagerly expands in error messages.
    let union_prefix = if action_indices.is_empty() {
        String::new()
    } else {
        let mut args = String::new();
        for (i, index) in action_indices.clone().enumerate() {
            if i > 0 {
                args.push_str(", ");
            }
            let _ = write!(args, "__svn_action_{index}");
        }
        format!("__svn_union({args}), ")
    };
    if tag_literal {
        let _ = write!(
            buf,
            "{indent}{{ svelteHTML.createElement(\"{tag_name}\", {union_prefix}{{"
        );
    } else {
        let _ = write!(
            buf,
            "{indent}{{ svelteHTML.createElement({tag_name}, {union_prefix}{{"
        );
    }
    let mut any = false;
    for attr in attributes {
        match attr {
            svn_parser::Attribute::Plain(p) => {
                if dom_attr_should_skip(p.name.as_str()) {
                    continue;
                }
                if !any {
                    buf.push_str("\n");
                    any = true;
                }
                emit_dom_plain_attr(buf, source, p, depth + 1);
            }
            svn_parser::Attribute::Expression(e) => {
                if dom_attr_should_skip(e.name.as_str()) {
                    continue;
                }
                if !any {
                    buf.push_str("\n");
                    any = true;
                }
                emit_dom_expression_attr(buf, source, e, depth + 1);
            }
            svn_parser::Attribute::Shorthand(s) => {
                if dom_attr_should_skip(s.name.as_str()) {
                    continue;
                }
                if !any {
                    buf.push_str("\n");
                    any = true;
                }
                emit_dom_shorthand_attr(buf, source, s, depth + 1);
            }
            svn_parser::Attribute::Spread(s) => {
                let Some(expr) =
                    source.get(s.expression_range.start as usize..s.expression_range.end as usize)
                else {
                    continue;
                };
                let trimmed = expr.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if !any {
                    buf.push_str("\n");
                    any = true;
                }
                let inner = "    ".repeat(depth + 1);
                let leading_ws = (expr.len() - expr.trim_start().len()) as u32;
                let start = s.expression_range.start + leading_ws;
                let end = start + trimmed.len() as u32;
                if s.is_attach {
                    // `{@attach EXPR}` (Svelte 5.29+): emit as a
                    // computed-symbol property so the arrow's parameter
                    // picks up the element's contextual type via the
                    // `[key: symbol]: Attachment<T> | …` index signature
                    // on `HTMLAttributes` in svelte/elements.d.ts.
                    let _ = write!(buf, "{inner}[Symbol(\"@attach\")]: ");
                } else {
                    let _ = write!(buf, "{inner}...");
                }
                buf.append_with_source(trimmed, svn_core::Range::new(start, end));
                let _ = writeln!(buf, ",");
            }
            // Directives (bind:, use:, class:, style:, transition:, …)
            // are handled outside createElement — by the existing bind/use
            // passes and by emit_dom_directive_checks.
            svn_parser::Attribute::Directive(_) => {}
        }
    }
    if any {
        let _ = writeln!(buf, "{indent}}});");
    } else {
        buf.push_str("}); ");
    }
}

/// Post-createElement directive checks for `class:` and `style:`
/// attributes. Emit each directive's value expression (or shorthand
/// identifier) as a bare statement inside the element's scoped block.
///
/// - `class:foo={cond}` / `class:foo` (shorthand) → `(cond);` or
///   `(foo);`. Type-checks the reference without constraining the
///   attribute slot.
/// - `style:prop={value}` / `style:color` (shorthand) → wraps the
///   value in `__svn_ensure_type(String, Number, value)` to validate
///   against the CSS-value union.
///
/// Mirrors upstream svelte2tsx's `Class.ts` and `StyleDirective.ts`.
pub(crate) fn emit_dom_directive_checks(
    buf: &mut EmitBuffer,
    source: &str,
    attributes: &[svn_parser::Attribute],
    depth: usize,
) {
    let indent = "    ".repeat(depth);
    for attr in attributes {
        let svn_parser::Attribute::Directive(d) = attr else {
            continue;
        };
        match d.kind {
            svn_parser::DirectiveKind::Class => match &d.value {
                Some(svn_parser::DirectiveValue::Expression {
                    expression_range, ..
                }) => {
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
                    let _ = write!(buf, "{indent}(");
                    buf.append_with_source(trimmed, svn_core::Range::new(start, end));
                    let _ = writeln!(buf, ");");
                }
                _ => {
                    // Shorthand `class:foo` — type-check `foo` as an
                    // identifier reference.
                    let _ = writeln!(buf, "{indent}({name});", name = d.name.as_str());
                }
            },
            // Style directives: wrap the value in
            // `__svn_ensure_type(String, Number, <value>)` so it type-
            // checks against `String | Number | null | undefined`.
            // Mirrors upstream svelte2tsx's `handleStyleDirective`.
            svn_parser::DirectiveKind::Style => match &d.value {
                Some(svn_parser::DirectiveValue::Expression {
                    expression_range, ..
                }) => {
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
                    let _ = write!(buf, "{indent}__svn_ensure_type(String, Number, ");
                    buf.append_with_source(trimmed, svn_core::Range::new(start, end));
                    let _ = writeln!(buf, ");");
                }
                Some(svn_parser::DirectiveValue::Quoted(av)) => {
                    emit_style_directive_template_literal(buf, source, av, &indent);
                }
                _ => {}
            },
            _ => {}
        }
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
fn emit_style_directive_template_literal(
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

/// Drop attributes that the svelte-jsx typings reject at the strict
/// interface but real Svelte allows: `data-*`, `aria-*`, CSS custom
/// properties, namespaced (`xml:lang`, `xlink:href`), the `this`
/// directive on `<svelte:element>`, the `slot=""` directive, and
/// React-style camelCase synonyms whose lowercase counterparts svelte-jsx
/// declares.
fn dom_attr_should_skip(name: &str) -> bool {
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
    if dom_attr_is_react_camelcase_synonym(name) {
        return true;
    }
    false
}

/// React-style camelCase HTML attribute names that have a lowercase
/// HTML5 counterpart. svelte-jsx declares only the lowercase form;
/// upstream silently drops these via source-map filtering.
fn dom_attr_is_react_camelcase_synonym(name: &str) -> bool {
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

/// Close the scoped block opened by `emit_dom_element_open`.
pub(crate) fn emit_dom_element_close(buf: &mut EmitBuffer, depth: usize) {
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}}}");
}

/// Open a `svelteHTML.createElement` scoped block for a
/// `<svelte:*>` element. Dispatches on `SvelteElementKind`:
///   - Body/Head/Window/Document/Options/Fragment: literal
///     `"svelte:<name>"` tag string. IntrinsicElements in our
///     shim has these as named keys.
///   - Element: `<svelte:element this={expr}>` — pass the `this`
///     expression verbatim as the first createElement arg so TS
///     checks the tag against IntrinsicElements keys.
///   - SelfRef/Component/Boundary/missing-this: skip the
///     createElement scope (not DOM elements). Open a bare
///     `{ }` block so child emit still scopes locals correctly.
pub(crate) fn emit_svelte_element_open(
    buf: &mut EmitBuffer,
    source: &str,
    s: &svn_parser::SvelteElement,
    depth: usize,
    action_indices: &std::ops::Range<usize>,
) {
    use svn_parser::SvelteElementKind::*;
    let indent = "    ".repeat(depth);
    match s.kind {
        Body | Head | Window | Document | Options | Fragment => {
            let tag = format!("svelte:{}", s.kind.as_str());
            emit_dom_element_open(
                buf,
                source,
                &tag,
                true,
                &s.attributes,
                depth,
                action_indices,
            );
        }
        Element => {
            // Find `this={expr}` among attributes.
            let this_expr = s.attributes.iter().find_map(|a| {
                let svn_parser::Attribute::Expression(e) = a else {
                    return None;
                };
                if e.name.as_str() != "this" {
                    return None;
                }
                source
                    .get(e.expression_range.start as usize..e.expression_range.end as usize)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            });
            match this_expr {
                Some(expr) => {
                    emit_dom_element_open(
                        buf,
                        source,
                        &format!("({expr})"),
                        false,
                        &s.attributes,
                        depth,
                        action_indices,
                    );
                }
                None => {
                    // Missing `this` — bare scope. Child emit still runs.
                    let _ = writeln!(buf, "{indent}{{");
                }
            }
        }
        SelfRef | Component | Boundary => {
            // Not a DOM element — bare scope for children.
            let _ = writeln!(buf, "{indent}{{");
        }
    }
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

fn emit_dom_plain_attr(
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

fn emit_dom_expression_attr(
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

fn emit_dom_shorthand_attr(
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

/// Map a static HTML/SVG tag name to a `HTMLElementTagNameMap['tag']`
/// / `SVGElementTagNameMap['tag']` indexed-access type. Dynamic
/// tags (empty string) fall back to `HTMLElement`. Unknown tag
/// names that aren't in either map would resolve through these
/// indexed accesses to `any` — acceptable, just means the check
/// stays lax for custom elements.
pub(crate) fn element_type_annotation(tag_name: &str) -> String {
    if tag_name.is_empty() {
        return "HTMLElement".to_string();
    }
    format!("HTMLElementTagNameMap['{tag_name}']")
}
