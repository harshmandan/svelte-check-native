//! Props-shape derivation for the render body and default-export
//! projection.
//!
//! Pulled out of `lib.rs` so the props-synthesis concern is a single
//! readable file. Two entry points are used by the main emit flow:
//!
//! - [`build_exports_object`] — assembles the `{ name: T; … }` object-
//!   type literal that backs `Awaited<ReturnType<typeof $$render>>['exports']`.
//! - [`inject_component_props_annotation`] — rewrites the user's
//!   `let { … } = $props()` destructure to carry our synthesised
//!   `$$ComponentProps` typedef.
//!
//! Plus the slot-defs literal builder [`build_slots_field_type`].

use std::fmt::Write;

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Expression, Statement, VariableDeclarator};

use crate::process_instance_script_content;
use crate::util::is_simple_js_identifier;

/// Build the component's `exports` object type — what consumers of
/// `bind:this={x}` see as `x.name` — the way upstream's
/// `ExportedNames.createExportsStr` does: every export that is not a
/// `let` (a legacy `export let` is a prop, not an instance member), plus
/// named `export { … }` lets in runes mode. Each member is required and
/// keyed by the name the component exposes. `None` when nothing
/// qualifies.
///
/// The text is embedded INSIDE `$$render`'s body
/// (`return { … exports: undefined as any as <text> }`), where
/// `typeof <local>` resolves against the body-local declaration; any
/// module-scope use goes through the
/// `Awaited<ReturnType<typeof $$render>>['exports']` projection.
pub(crate) fn build_exports_object(
    split: Option<&process_instance_script_content::SplitScript>,
    runes_mode: bool,
    uses_accessors: bool,
) -> Option<String> {
    let s = split?;
    // Accessors make every export, props included, an instance member;
    // runes mode has no accessors.
    let all = uses_accessors && !runes_mode;
    let mut members = s
        .export_type_infos
        .iter()
        .filter(|info| all || !info.is_let || (runes_mode && info.is_named_export))
        .peekable();
    members.peek()?;
    let mut buf = String::from("{ ");
    for info in members {
        buf.push_str(info.exported_as.as_deref().unwrap_or(&info.name));
        buf.push_str(": ");
        match &info.type_source {
            Some(t) => buf.push_str(t),
            None => {
                buf.push_str("typeof ");
                buf.push_str(info.name.as_str());
            }
        }
        buf.push_str("; ");
    }
    buf.push('}');
    Some(buf)
}

/// Whether `<svelte:options accessors>` turns accessors on, read the way
/// svelte2tsx's `handleSvelteOptions` reads it: a bare attribute is on;
/// a `{…}` value is on when it is a truthy literal; a text value leaves
/// the setting alone. The last `accessors` attribute decides.
pub(crate) fn uses_accessors(fragment: &svn_parser::Fragment, source: &str) -> bool {
    use svn_parser::{AttrValuePart, Attribute, Node, SvelteElementKind};
    let truthy_literal = |range: svn_core::Range| {
        let text = source
            .get(range.start as usize..range.end as usize)
            .unwrap_or("")
            .trim();
        !matches!(
            text,
            "false" | "0" | "null" | "undefined" | "\"\"" | "''" | "``"
        ) && (text == "true"
            || text.parse::<f64>().is_ok_and(|n| n != 0.0)
            || text.starts_with(['"', '\'', '`']))
    };
    let mut on = false;
    for node in &fragment.nodes {
        let Node::SvelteElement(se) = node else {
            continue;
        };
        if se.kind != SvelteElementKind::Options {
            continue;
        }
        for attr in &se.attributes {
            match attr {
                Attribute::Plain(p) if p.name.as_str() == "accessors" => match &p.value {
                    None => on = true,
                    Some(v) => {
                        if let Some(AttrValuePart::Expression {
                            expression_range, ..
                        }) = v.parts.first()
                        {
                            on = truthy_literal(*expression_range);
                        }
                    }
                },
                Attribute::Expression(e) if e.name.as_str() == "accessors" => {
                    on = truthy_literal(e.expression_range);
                }
                Attribute::Shorthand(s) if s.name.as_str() == "accessors" => on = false,
                _ => {}
            }
        }
    }
    on
}

/// Build the body of a JSDoc `@typedef <body> $$ComponentProps` from a
/// `$props()` destructure. Returns `Some("{name: any, opt?: string}")`
/// (a complete object-type typespec including the outer `{}`).
///
/// Each prop-surface entry (not `...rest`, not `local_only`) maps to:
///   - `key: any` for required (no default, not $bindable)
///   - `key?: <inferred>` for optional, where `<inferred>` is the
///     literal-type derived from the default expression (string for
///     `= ''`, `Function` for `= () => {}`, `Record<string, any>`
///     for `= {}`, etc.); falls back to `any` for unrecognised
///     default expressions.
///
/// Mirrors upstream svelte2tsx's hard-mode synthesis
/// (handle$propsRune) including its `withUnknown` widening: any
/// non-simple element — `...rest`, a nested pattern, a non-identifier
/// key — appends `& Record<string, any>` so extra props at consumers
/// don't fire a false excess-property error; when NO simple element
/// remains the whole type collapses to bare `Record<string, any>`.
/// Nested-pattern leaves never surface as prop keys. Returns `None`
/// when there's nothing to synthesise (empty destructure, or a
/// non-object `$props()` binding — upstream's emission gate
/// `props.length > 0 || withUnknown` is false there and no typedef is
/// emitted).
pub(crate) fn synthesise_js_props_typedef_body(
    props_info: &svn_analyze::PropsInfo,
) -> Option<String> {
    let mut body = String::from("{");
    let mut first = true;
    for entry in &props_info.destructures {
        if entry.is_rest || entry.local_only {
            // Covered by the `withUnknown` widening below — upstream
            // pushes no prop key for these elements.
            continue;
        }
        let key = entry.prop_key.as_str();
        if !first {
            body.push_str(", ");
        }
        first = false;
        let key_text = if is_simple_js_identifier(key) {
            key.to_string()
        } else {
            format!("\"{}\"", key.replace('"', "\\\""))
        };
        let optional = if entry.has_default { "?" } else { "" };
        let value_type = entry
            .default_type_text
            .as_deref()
            .filter(|_| entry.has_default)
            .unwrap_or("any");
        let _ = write!(body, "{key_text}{optional}: {value_type}");
    }
    body.push('}');
    if first {
        // No prop-surface entries. Bare widening when the pattern had
        // non-simple elements; otherwise nothing to synthesise.
        return props_info
            .props_with_unknown
            .then(|| "Record<string, any>".to_string());
    }
    if props_info.props_with_unknown {
        body.push_str(" & Record<string, any>");
    }
    Some(body)
}

/// Inject `: $$ComponentProps` onto the destructure pattern of an
/// untyped top-level `let/const { … } = $props()` declaration.
///
/// Mirrors upstream svelte2tsx's `ExportedNames.ts:388` — when a
/// `$$ComponentProps` type alias is synthesized at module scope, the
/// `$props()` destructure gets the matching annotation so each
/// destructured local (`data`, `form`, etc.) picks up the declared
/// type rather than falling through to `$props()`'s loose return.
///
/// Returns `content` unchanged when:
/// - No `let/const { … } = $props()` at top level, OR
/// - The pattern already has a type annotation (user-written), OR
/// - Parse fails (conservative: don't break a valid script).
///
/// The rewrite is AST-driven to avoid false positives on comment /
/// string-literal content that happens to include `= $props()`.
pub(crate) fn inject_component_props_annotation(
    content: &str,
    lang: svn_parser::ScriptLang,
) -> String {
    let alloc = Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, content, lang);
    let mut action: Option<AnnotationAction> = None;
    for stmt in &parsed.program.body {
        let decl = match stmt {
            Statement::VariableDeclaration(d) => d,
            _ => continue,
        };
        for declarator in &decl.declarations {
            if let Some(a) = annotation_action(declarator) {
                // Use the FIRST $props destructure — upstream only
                // recognises one.
                action = Some(a);
                break;
            }
        }
        if action.is_some() {
            break;
        }
    }
    let Some(action) = action else {
        return content.to_string();
    };
    let mut out = String::with_capacity(content.len() + 32);
    match action {
        AnnotationAction::ReplaceTypeArgument { start, end } => {
            // Keep the line count, as for the annotation below. A type
            // argument on a function that takes none is an error, and
            // upstream's marks keep it from being reported.
            let dropped_newlines = content[start..end].matches('\n').count();
            out.push_str(&content[..start]);
            out.push_str("/*svn:ignore_start*/$$ComponentProps");
            for _ in 0..dropped_newlines {
                out.push('\n');
            }
            out.push_str("/*svn:ignore_end*/");
            out.push_str(&content[end..]);
        }
        AnnotationAction::Insert(pos) => {
            out.push_str(&content[..pos]);
            out.push_str(": $$ComponentProps");
            out.push_str(&content[pos..]);
        }
        AnnotationAction::Replace { start, end } => {
            // Replace the user's literal annotation with a single
            // `$$ComponentProps` reference wrapped in ignore markers.
            // Upstream svelte2tsx does the same swap (see
            // `ExportedNames.ts`'s `$props` rewrite); the ignore
            // markers tell svelte-check's diagnostic mapper to drop
            // any tsgo errors INSIDE the marker span, since the
            // rewritten alias name has no source-position
            // correspondence to the user's original literal.
            //
            // Preserve the source's line count over the replaced span:
            // the original annotation may straddle multiple lines (a
            // multi-line type literal), and the script-body line_map
            // assumes 1:1 source/overlay line correspondence. Without
            // padding, every declaration after the rewrite drifts by
            // (literal-line-count - 1) lines in mapped diagnostics.
            // Pad newlines INSIDE the marker span so the line count is
            // preserved.
            //
            // The `Ω`-spelled markers are emit-shape parity with
            // upstream svelte2tsx, which brackets the same replacement
            // the same way. They are NOT the markers our own diagnostic
            // mapper scans for — that is the ASCII
            // `IGNORE_START_MARKER` / `IGNORE_END_MARKER` pair — so
            // nothing here suppresses diagnostics. What keeps positions
            // right is the line-count parity alone.
            //
            // Do not "unify" the two spellings by teaching the scanner
            // this one: a user's own error inside a `$props()` type
            // annotation would start disappearing.
            let dropped_newlines = content[start..end].matches('\n').count();
            out.push_str(&content[..start]);
            out.push_str(": /*\u{03A9}ignore_start\u{03A9}*/$$ComponentProps");
            for _ in 0..dropped_newlines {
                out.push('\n');
            }
            out.push_str("/*\u{03A9}ignore_end\u{03A9}*/");
            out.push_str(&content[end..]);
        }
    }
    out
}

enum AnnotationAction {
    Insert(usize),
    Replace { start: usize, end: usize },
    ReplaceTypeArgument { start: usize, end: usize },
}

fn annotation_action(declarator: &VariableDeclarator<'_>) -> Option<AnnotationAction> {
    use oxc_span::GetSpan;
    let init = declarator.init.as_ref()?;
    let Expression::CallExpression(call) = init else {
        return None;
    };
    let Expression::Identifier(callee_id) = &call.callee else {
        return None;
    };
    if callee_id.name != "$props" {
        return None;
    }
    if let Some(args) = &call.type_arguments {
        // `$props<{ … }>()`, destructured or not: upstream moves a
        // literal type argument into the `$$ComponentProps` alias and
        // leaves the alias name, marked generated, as the argument. A
        // named type stays where it is.
        let arg = args.params.first()?;
        if matches!(arg, oxc_ast::ast::TSType::TSTypeReference(_)) {
            return None;
        }
        return Some(AnnotationAction::ReplaceTypeArgument {
            start: arg.span().start as usize,
            end: arg.span().end as usize,
        });
    }
    // Without a type argument, only a destructure gets the annotation.
    let BindingPattern::ObjectPattern(obj) = &declarator.id else {
        return None;
    };
    // CASE A — user wrote `let { … }: { lit } = $props()`. Replace
    // the literal annotation with `$$ComponentProps` (wrapped in
    // ignore markers to drop tsgo errors inside). This collapses a
    // multi-line literal to a single token — matching upstream's
    // line-count parity and eliminating downstream position drift on
    // the destructure-following declarations.
    if let Some(annot) = &declarator.type_annotation {
        let start = annot.span.start as usize;
        let end = annot.span.end as usize;
        return Some(AnnotationAction::Replace { start, end });
    }
    // CASE B — no existing annotation. Splice after the destructure
    // pattern's closing `}`.
    Some(AnnotationAction::Insert(obj.span.end as usize))
}

/// Build the slot-defs object literal for the render body.
///
/// Empty slot list → `undefined as any as {}`.
/// Otherwise → `{ 'name': { attr: (expr), ... }, ... }`.
///
/// TS infers the slots type from the literal (each prop's type is
/// the inferred type of its expression). Consumer-side
/// `inst.$$slot_def.default.nodes` then has the right type.
///
/// Mirrors upstream svelte2tsx's `slotsAsDef` builder in
/// `createRenderFunction.ts:125-133`.
pub(crate) fn write_slots_field_type(
    out: &mut String,
    source: &str,
    slot_defs: &[svn_analyze::SlotDef],
    is_ts: bool,
) {
    if slot_defs.is_empty() {
        // JS overlays can't carry the `as` cast; a bare `{}` literal
        // types identically for the empty-slots case.
        out.push_str(if is_ts {
            "undefined as any as {}"
        } else {
            "{}"
        });
        return;
    }
    out.push_str("{ ");
    let mut first_slot = true;
    for def in slot_defs {
        if !first_slot {
            out.push_str(", ");
        }
        first_slot = false;
        out.push('\'');
        out.push_str(def.slot_name.as_str());
        out.push_str("': { ");
        let mut first_attr = true;
        for attr in &def.attrs {
            if !first_attr {
                out.push_str(", ");
            }
            first_attr = false;
            match attr {
                svn_analyze::SlotAttr::Prop { name, expr } => {
                    out.push_str(name.as_str());
                    out.push_str(": ");
                    write_slot_attr_expr(out, source, expr, is_ts);
                }
                svn_analyze::SlotAttr::Spread { expr } => {
                    // `<slot {...row}>` carries the spread's expression
                    // through `write_slot_attr_expr_inner`. The Type-
                    // form arm (round-8 #1) emits `undefined as any as
                    // (T)` so the wrap becomes
                    // `...(undefined as any as (T))` — syntactically
                    // valid AND projects the right element shape.
                    out.push_str("...(");
                    write_slot_attr_expr_inner(out, source, expr, is_ts);
                    out.push(')');
                }
            }
        }
        out.push_str(" }");
    }
    out.push_str(" }");
}

/// Write a slot-attr expression value with the outer parens / cast
/// shape per its variant: `Resolved::Type` becomes `undefined as any
/// as (T)` (JS overlays: the JSDoc-cast equivalent); everything else
/// is wrapped in `(…)`.
fn write_slot_attr_expr(
    out: &mut String,
    source: &str,
    expr: &svn_analyze::SlotAttrExpr,
    is_ts: bool,
) {
    if let svn_analyze::SlotAttrExpr::Resolved(svn_analyze::ResolvedSlotExpr::Type(t)) = expr {
        write_type_valued_expr(out, t, is_ts);
        return;
    }
    out.push('(');
    write_slot_attr_expr_inner(out, source, expr, is_ts);
    out.push(')');
}

/// Emit an expression whose TYPE is `t` with no runtime value behind
/// it: `undefined as any as (T)` in TS overlays, the JSDoc double-cast
/// `/** @type {T} */ (/** @type {any} */ (null))` in JS overlays
/// (single-cast `null → T` would fire TS2352 under strictNullChecks).
fn write_type_valued_expr(out: &mut String, t: &str, is_ts: bool) {
    if is_ts {
        out.push_str("undefined as any as (");
        out.push_str(t);
        out.push(')');
    } else {
        out.push_str("/** @type {");
        out.push_str(t);
        out.push_str("} */ (/** @type {any} */ (null))");
    }
}

fn write_slot_attr_expr_inner(
    out: &mut String,
    source: &str,
    expr: &svn_analyze::SlotAttrExpr,
    is_ts: bool,
) {
    match expr {
        // Range form — slice the original source. A `get`
        // miss means the caller passed a source the walker
        // didn't see; fall through with empty parens to
        // preserve the output shape.
        svn_analyze::SlotAttrExpr::Range(range) => {
            if let Some(text) = source.get(range.start as usize..range.end as usize) {
                out.push_str(text);
            }
        }
        // Shorthand stored the identifier inline.
        svn_analyze::SlotAttrExpr::Shorthand(ident) => {
            out.push_str(ident.as_str());
        }
        // Literal text from `<slot foo="bar">` — emit as a
        // double-quoted TS string. Escape `"` and `\` so the
        // emitted token-stream parses cleanly even when the
        // user's literal contains them.
        svn_analyze::SlotAttrExpr::Literal(text) => {
            out.push('"');
            for ch in text.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    _ => out.push(ch),
                }
            }
            out.push('"');
        }
        svn_analyze::SlotAttrExpr::Resolved(svn_analyze::ResolvedSlotExpr::Value(v)) => {
            out.push_str(v);
        }
        svn_analyze::SlotAttrExpr::Resolved(svn_analyze::ResolvedSlotExpr::Type(t)) => {
            // Round-8 follow-up #1: this arm is only reached via the
            // Spread case (`<slot {...row}>` whose `row` is shadowed
            // and rewrites to a type-form). The Prop case strips
            // Type() at the outer `write_slot_attr_expr` and never
            // reaches this inner writer for Type. Pre-fix this arm
            // wrote nothing — the spread emitted as `...()` (empty),
            // which is a syntax error inside an object literal.
            // Emit a typed cast so the spread becomes
            // `...(undefined as any as (T))` (JS: the JSDoc-cast
            // equivalent).
            write_type_valued_expr(out, t, is_ts);
        }
    }
}
