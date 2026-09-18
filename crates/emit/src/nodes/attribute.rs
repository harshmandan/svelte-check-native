//! Plain / expression / shorthand attribute emission for DOM elements.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Attribute.ts`.
//!
//! Each handler emits one entry of the `svelteHTML.createElement("tag", {
//! …entries })` literal that [`crate::nodes::element::emit_dom_element_open`]
//! is currently building. The attribute-skip table (`should_skip`) is
//! also here so the dispatcher consults a single source of truth.

use std::borrow::Cow;
use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// SVG attributes whose name carries case that must be PRESERVED.
/// Upstream `transformAttributeCase` (Attribute.ts:113-124) lowercases
/// every DOM-element attribute name except those in `svgattributes.ts`,
/// custom elements, and Svelte-5 `on*` handlers. Since our transform
/// only ever lowercases, only the uppercase-bearing SVG names need
/// protecting (all-lowercase / hyphenated SVG names lowercase to
/// themselves). Sorted for `binary_search`.
const SVG_PRESERVE_CASE: &[&str] = &[
    "allowReorder",
    "attributeName",
    "attributeType",
    "autoReverse",
    "baseFrequency",
    "baseProfile",
    "calcMode",
    "clipPathUnits",
    "contentScriptType",
    "contentStyleType",
    "diffuseConstant",
    "edgeMode",
    "externalResourcesRequired",
    "filterRes",
    "filterUnits",
    "glyphRef",
    "gradientTransform",
    "gradientUnits",
    "kernelMatrix",
    "kernelUnitLength",
    "keyPoints",
    "keySplines",
    "keyTimes",
    "lengthAdjust",
    "limitingConeAngle",
    "markerHeight",
    "markerUnits",
    "markerWidth",
    "maskContentUnits",
    "maskUnits",
    "numOctaves",
    "pathLength",
    "patternContentUnits",
    "patternTransform",
    "patternUnits",
    "pointsAtX",
    "pointsAtY",
    "pointsAtZ",
    "preserveAlpha",
    "preserveAspectRatio",
    "primitiveUnits",
    "refX",
    "refY",
    "repeatCount",
    "repeatDur",
    "requiredExtensions",
    "requiredFeatures",
    "specularConstant",
    "specularExponent",
    "spreadMethod",
    "startOffset",
    "stdDeviation",
    "stitchTiles",
    "surfaceScale",
    "systemLanguage",
    "tableValues",
    "targetX",
    "targetY",
    "textLength",
    "viewBox",
    "viewTarget",
    "xChannelSelector",
    "yChannelSelector",
    "zoomAndPan",
];

/// Lowercase a DOM-element attribute name to match svelte-jsx's
/// intrinsic-element typings, mirroring upstream svelte2tsx's
/// `transformAttributeCase` (Attribute.ts:113-124). The name is kept
/// verbatim when:
///   - it's a case-sensitive SVG attribute (`viewBox`, `preserveAspectRatio`),
///     or
///   - it's an `on*` handler — upstream preserves these under Svelte 5.
///     We always preserve them: Svelte 4 events are `on:` *directives*
///     (parsed separately), so a plain `on*` attribute only reaches here
///     in Svelte-5 components.
///
/// The custom-element carve-out is the caller's responsibility (it passes
/// `should_lowercase = false`).
///
/// `should_lowercase` is true only for static DOM elements that are NOT
/// custom elements (mirrors upstream's `element instanceof Element &&
/// !element.isCustomElement()`); dynamic `<svelte:element>`, components,
/// and custom elements pass `false` and keep names verbatim. The
/// transform only lowercases, so it never changes the name's byte length
/// — source-position mapping via `name_range` stays 1:1.
pub(crate) fn transform_attribute_case(name: &str, should_lowercase: bool) -> Cow<'_, str> {
    if !should_lowercase || !name.bytes().any(|b| b.is_ascii_uppercase()) {
        return Cow::Borrowed(name); // disabled, or already lowercase — no-op
    }
    // `on*` handlers keep their case under Svelte 5; SVG case-sensitive
    // names (`viewBox`, …) are never folded.
    if name.starts_with("on") || SVG_PRESERVE_CASE.binary_search(&name).is_ok() {
        return Cow::Borrowed(name);
    }
    Cow::Owned(name.to_ascii_lowercase())
}

/// Attributes `Attribute.ts` does not pass through to the element's
/// attribute object: `this` on `<svelte:element>`, and a text `slot`.
/// CSS custom properties (`--x={…}`) are passed like any attribute and
/// checked against the element's typings. Namespaced attributes (`xml:lang`, `xlink:href`) are NOT
/// dropped — they flow through `createElement` with a quoted key so they
/// reproduce upstream's diagnostics. React-style camelCase synonyms
/// (`className`, `tabIndex`, …) are likewise NOT dropped — they're
/// lowercased by [`transform_attribute_case`] so tsgo surfaces the same
/// value-check (`tabindex`) or unknown-attribute error (`classname`) as
/// upstream, instead of being silently accepted.
///
/// `data-*` is NOT skipped — it's wrapped in `...__svn_empty({...})`
/// at emit time so the value expression stays referenced (suppresses
/// TS6133 on identifiers that only appear in `data-foo={expr}`
/// attributes). Mirrors upstream svelte2tsx's `Attribute.ts:86-94`.
/// `data-sveltekit-*` is the carve-out: those are typed in svelte-jsx
/// directly so the wrap would add noise — pass them through unwrapped.
pub(crate) fn should_skip(
    name: &str,
    value: Option<&svn_parser::AttrValue>,
    svelte_element: bool,
) -> bool {
    // `this` on `<svelte:element>` is its tag, not an attribute.
    if name == "this" {
        return svelte_element;
    }
    // A text `slot="…"` places the element in a component's named slot
    // (`Attribute.ts` hands it to the slot handling); as a DOM attribute
    // it would only be a valid string anyway.
    if name == "slot" {
        return matches!(
            value.map(|v| v.parts.as_slice()),
            Some([svn_parser::AttrValuePart::Text { .. }])
        );
    }
    false
}

/// True when the attribute should be wrapped in
/// `...__svn_empty({...})` at emit time. Matches upstream
/// `Attribute.ts:86` predicate exactly: any `data-*` attribute that
/// isn't a `data-sveltekit-*` (those are typed by svelte-jsx
/// directly).
pub(crate) fn needs_data_attr_wrap(name: &str) -> bool {
    name.starts_with("data-") && !name.starts_with("data-sveltekit-")
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

/// JavaScript's `Number(text)`, `None` for `NaN`: surrounding
/// whitespace ignored, empty text is 0, unsigned `0x`/`0o`/`0b`
/// integers, and decimal literals or `Infinity` with an optional sign.
fn js_number(text: &str) -> Option<f64> {
    let t = text.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if t.is_empty() {
        return Some(0.0);
    }
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if t.len() > 2 && t[..2].eq_ignore_ascii_case(prefix) {
            return u64::from_str_radix(&t[2..], radix).ok().map(|n| n as f64);
        }
    }
    let unsigned = t.strip_prefix(['+', '-']).unwrap_or(t);
    if unsigned == "Infinity" {
        return Some(if t.starts_with('-') {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        });
    }
    let decimal = unsigned
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !decimal || !unsigned.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse::<f64>().ok()
}

/// Write an attribute's quoted key and its `:`. svelte2tsx keeps the
/// name's source range and writes the quotes around it (the opening one
/// over the name's first character), so a diagnostic on the key spans
/// the name. Its closing quote follows the name directly when a value
/// follows, and replaces the name's last character when none does, so
/// a valueless attribute's range ends one character short.
pub(crate) fn write_attribute_key(
    buf: &mut EmitBuffer,
    key: &str,
    name_range: svn_core::Range,
    has_value: bool,
) {
    let (start, end) = (name_range.start, name_range.end);
    if key.len() != (end - start) as usize || key.is_empty() || key.contains(['"', '\\']) {
        buf.append_with_source(&format!("\"{key}\""), name_range);
        buf.push(':');
        return;
    }
    buf.append_with_source("\"", svn_core::Range::new(start, start + 1));
    buf.append_with_source(key, name_range);
    let close = if has_value {
        svn_core::Range::new(end, end + 1)
    } else {
        svn_core::Range::new(end - 1, end)
    };
    buf.append_with_source("\":", close);
}

pub(crate) fn emit_plain(
    buf: &mut EmitBuffer,
    source: &str,
    p: &svn_parser::PlainAttr,
    depth: usize,
    should_lowercase: bool,
    parent_is_element: bool,
) {
    let indent = "    ".repeat(depth);
    let name = p.name.as_str();
    let name_range = svn_core::Range::new(p.range.start, p.range.start + name.len() as u32);
    // Lowercase the emitted key for DOM elements (transform preserves
    // byte length, so `name_range` still maps the source name).
    let key_name = transform_attribute_case(name, should_lowercase);
    let wrap = needs_data_attr_wrap(name);
    let (key_prefix, line_suffix) = if wrap {
        ("...__svn_empty({", "}),")
    } else {
        ("", ",")
    };
    match &p.value {
        None => {
            // Boolean attribute: `<input required>` → `"required": true,`
            // `<div popover>` carve-out: upstream emits `"": ""`.
            // For data-* boolean attrs, upstream uses `__sveltets_2_any()`
            // as the value placeholder (Attribute.ts:91). Match it.
            let value = if wrap {
                "__svn_any()"
            } else if name == "popover" {
                "\"\""
            } else {
                "true"
            };
            buf.push_str(&indent);
            buf.push_str(key_prefix);
            write_attribute_key(buf, &key_name, name_range, false);
            let _ = writeln!(buf, " {value}{line_suffix}");
        }
        Some(v) => {
            // numberOnlyAttributes: a plain number text on an element is
            // written as the number itself (`!isNaN(value)`).
            if let [svn_parser::AttrValuePart::Text { range }] = v.parts.as_slice() {
                let content = range.slice(source);
                if parent_is_element
                    && is_number_only_attr(&name.to_ascii_lowercase())
                    && !content.is_empty()
                    && !content.trim_end().ends_with('}')
                    && js_number(content).is_some()
                {
                    buf.push_str(&indent);
                    buf.push_str(key_prefix);
                    write_attribute_key(buf, &key_name, name_range, true);
                    let _ = writeln!(buf, " {content}{line_suffix}");
                    return;
                }
            }
            if plain_value_is_dropped(source, v) {
                return;
            }
            buf.push_str(&indent);
            buf.push_str(key_prefix);
            write_attribute_key(buf, &key_name, name_range, true);
            buf.push(' ');
            emit_plain_value(buf, source, v);
            let _ = writeln!(buf, "{line_suffix}");
        }
    }
}

/// True when a plain attribute's value would emit nothing: a single
/// `{expr}` interpolation whose expression is empty or whitespace-only
/// (or whose byte range doesn't slice cleanly). Callers that write a
/// key or separator before the value must gate on this and drop the
/// whole attribute — [`emit_plain_value`] emits zero bytes for it.
pub(crate) fn plain_value_is_dropped(source: &str, v: &svn_parser::AttrValue) -> bool {
    if let [
        svn_parser::AttrValuePart::Expression {
            expression_range, ..
        },
    ] = v.parts.as_slice()
    {
        source
            .get(expression_range.start as usize..expression_range.end as usize)
            .is_none_or(|expr| expr.trim().is_empty())
    } else {
        false
    }
}

/// Emit a plain attribute's value as a standalone JS expression.
/// Mirrors upstream `Attribute.ts`'s value handling, shared by the
/// element/component attribute entries and the `<slot>` prop check:
///
///   - no parts (`attr=""`) → `""`
///   - single text part → backtick template literal (newline-safe;
///     backslashes and backticks escaped)
///   - single `{expr}` part → `(expr)` with a token-map anchor on the
///     expression so diagnostics inside it map back to source
///   - mixed text + interpolations → one template literal with
///     `${expr}` holes, each hole token-mapped
///
/// Emits nothing when [`plain_value_is_dropped`] is true — callers
/// that write a prefix first must gate on it.
pub(crate) fn emit_plain_value(buf: &mut EmitBuffer, source: &str, v: &svn_parser::AttrValue) {
    match v.parts.as_slice() {
        [] => buf.push_str("\"\""),
        [svn_parser::AttrValuePart::Text { range }] => {
            let content = range.slice(source);
            let escaped = content.replace('\\', "\\\\").replace('`', "\\`");
            buf.push('`');
            buf.push_str(&escaped);
            buf.push('`');
        }
        [
            svn_parser::AttrValuePart::Expression {
                expression_range, ..
            },
        ] => {
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
            let (open, close) = value_parens(trimmed);
            buf.push_str(open);
            buf.append_with_source(trimmed, svn_core::Range::new(start, end));
            buf.push_str(close);
        }
        parts => {
            // Multi-part (text + interpolations). Template literal
            // with `${expr}` placeholders so the whole attribute
            // binds as a string. Follows upstream Attribute.ts's
            // multi-value branch.
            buf.push('`');
            for part in parts {
                match part {
                    svn_parser::AttrValuePart::Text { range } => {
                        let content = range.slice(source);
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
            buf.push('`');
        }
    }
}

pub(crate) fn emit_expression(
    buf: &mut EmitBuffer,
    source: &str,
    e: &svn_parser::ExpressionAttr,
    depth: usize,
    should_lowercase: bool,
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
    let wrap = needs_data_attr_wrap(name);
    let (key_prefix, line_suffix) = if wrap {
        ("...__svn_empty({", "}),")
    } else {
        ("", ",")
    };
    buf.push_str(&indent);
    buf.push_str(key_prefix);
    let name_range = svn_core::Range::new(e.range.start, e.range.start + name.len() as u32);
    let key_name = transform_attribute_case(name, should_lowercase);
    write_attribute_key(buf, &key_name, name_range, true);
    let (open, close) = value_parens(trimmed);
    buf.push_str(" ");
    buf.push_str(open);
    buf.append_with_source(trimmed, svn_core::Range::new(start, end));
    buf.push_str(close);
    // svelte2tsx writes the separator after a value over the
    // attribute's closing `}`, so an error the value leaves open (a
    // bare comma sequence, say) is reported at that brace.
    let brace = source
        .get(e.expression_range.end as usize..)
        .and_then(|rest| rest.find('}'))
        .map(|at| e.expression_range.end + at as u32);
    match (brace, line_suffix.strip_prefix('}')) {
        (Some(at), None) => {
            buf.append_with_source(line_suffix, svn_core::Range::new(at, at + 1));
            buf.push_str("\n");
        }
        _ => {
            let _ = writeln!(buf, "{line_suffix}");
        }
    }
}

/// Parentheses around an attribute value: every value is wrapped except
/// a comma sequence, which svelte2tsx leaves bare (see
/// [`crate::util::is_sequence_expression`]).
pub(crate) fn value_parens(expr: &str) -> (&'static str, &'static str) {
    if crate::util::is_sequence_expression(expr) {
        ("", "")
    } else {
        ("(", ")")
    }
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
    let name_range = svn_core::Range::new(name_start, name_end);
    buf.push_str(&indent);
    // `{foo}` is written as-is, never case-folded (`Attribute.ts`).
    buf.append_with_source(name, name_range);
    buf.push_str(",\n");
}
