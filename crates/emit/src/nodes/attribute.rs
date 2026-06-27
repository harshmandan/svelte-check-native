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

/// Drop attributes that the svelte-jsx typings reject at the strict
/// interface but real Svelte allows: `aria-*`, CSS custom properties,
/// the `this` directive on `<svelte:element>`, and the `slot=""`
/// directive. Namespaced attributes (`xml:lang`, `xlink:href`) are NOT
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
pub(crate) fn should_skip(name: &str) -> bool {
    if name.starts_with("aria-") {
        return true;
    }
    if name.starts_with("--") {
        return true;
    }
    if name == "this" {
        return true;
    }
    if name == "slot" {
        return true;
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

pub(crate) fn emit_plain(
    buf: &mut EmitBuffer,
    source: &str,
    p: &svn_parser::PlainAttr,
    depth: usize,
    should_lowercase: bool,
) {
    let indent = "    ".repeat(depth);
    let name = p.name.as_str();
    let name_range = svn_core::Range::new(p.range.start, p.range.start + name.len() as u32);
    // Lowercase the emitted key for DOM elements (transform preserves
    // byte length, so `name_range` still maps the source name).
    let key_name = transform_attribute_case(name, should_lowercase);
    let key_text = format!("\"{key_name}\"");
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
            buf.append_with_source(&key_text, name_range);
            let _ = writeln!(buf, ": {value}{line_suffix}");
        }
        Some(v) if v.parts.is_empty() => {
            buf.push_str(&indent);
            buf.push_str(key_prefix);
            buf.append_with_source(&key_text, name_range);
            let _ = writeln!(buf, ": \"\"{line_suffix}");
        }
        Some(v) if v.parts.len() == 1 => {
            match &v.parts[0] {
                svn_parser::AttrValuePart::Text { range } => {
                    let content = range.slice(source);
                    // numberOnlyAttributes carve-out: emit bare number
                    // literal when the text parses as a number.
                    //
                    // Upstream's test is `!isNaN(Number(x))`, so reject
                    // only the textual values Rust's `parse::<f64>`
                    // accepts that JS `Number()` does not: the `nan` /
                    // `inf` spellings and the lowercase `infinity`
                    // form. JS does accept exactly-cased `Infinity`
                    // (and signed variants), which become bare global
                    // `Infinity` — keep those as numbers.
                    let t = content.trim();
                    let parses_as_number = t.parse::<f64>().is_ok()
                        && !t.eq_ignore_ascii_case("nan")
                        && !t.eq_ignore_ascii_case("inf")
                        && (t == "Infinity"
                            || t == "+Infinity"
                            || t == "-Infinity"
                            || !t.eq_ignore_ascii_case("infinity"));
                    if is_number_only_attr(&name.to_ascii_lowercase())
                        && !content.is_empty()
                        && parses_as_number
                    {
                        buf.push_str(&indent);
                        buf.push_str(key_prefix);
                        buf.append_with_source(&key_text, name_range);
                        let _ = writeln!(buf, ": {}{line_suffix}", content.trim());
                        return;
                    }
                    let escaped = content.replace('\\', "\\\\").replace('`', "\\`");
                    buf.push_str(&indent);
                    buf.push_str(key_prefix);
                    buf.append_with_source(&key_text, name_range);
                    let _ = writeln!(buf, ": `{escaped}`{line_suffix}");
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
                    buf.push_str(key_prefix);
                    buf.append_with_source(&key_text, name_range);
                    buf.push_str(": (");
                    buf.append_with_source(trimmed, svn_core::Range::new(start, end));
                    let _ = writeln!(buf, "){line_suffix}");
                }
            }
        }
        Some(v) => {
            // Multi-part (text + interpolations). Template literal
            // with `${expr}` placeholders so the whole attribute
            // binds as a string. Follows upstream Attribute.ts's
            // multi-value branch.
            buf.push_str(&indent);
            buf.push_str(key_prefix);
            buf.append_with_source(&key_text, name_range);
            buf.push_str(": `");
            for part in &v.parts {
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
            let _ = writeln!(buf, "`{line_suffix}");
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
    buf.append_with_source(&format!("\"{key_name}\""), name_range);
    buf.push_str(": (");
    buf.append_with_source(trimmed, svn_core::Range::new(start, end));
    let _ = writeln!(buf, "){line_suffix}");
}

pub(crate) fn emit_shorthand(
    buf: &mut EmitBuffer,
    source: &str,
    s: &svn_parser::ShorthandAttr,
    depth: usize,
    should_lowercase: bool,
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
    // `{foo}` shorthand means `foo={foo}`. When the attribute name must
    // lowercase for a DOM element, the key and the value variable
    // diverge (`tabIndex` var → `tabindex` key), so emit the explicit
    // `"tabindex": (tabIndex)` form instead of object shorthand.
    match transform_attribute_case(name, should_lowercase) {
        Cow::Owned(lowered) => {
            let _ = write!(buf, "\"{lowered}\": (");
            buf.append_with_source(name, name_range);
            buf.push_str("),\n");
        }
        Cow::Borrowed(_) => {
            buf.append_with_source(name, name_range);
            buf.push_str(",\n");
        }
    }
}
