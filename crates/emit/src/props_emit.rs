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

/// Build the `{ name: sig; ... }` object-type text for each
/// `export function` / `export const` / `export let` that process_instance_script_content
/// surfaced. Consumed in two places:
///   - the render body's `return { exports: undefined as any as (…) }`
///     where body-local refs (`typeof handler`, `$$Props['x']`) resolve
///     inside `$$render`'s own scope.
///   - for non-class-wrapper arms, intersected into the default-export's
///     SvelteComponent type directly (may fire TS2304 for body-local
///     refs — rare and acceptable; class-wrapper arms take the other
///     path and avoid it entirely).
pub(crate) fn build_exports_object(
    split: Option<&process_instance_script_content::SplitScript>,
) -> Option<String> {
    let s = split?;
    if s.export_type_infos.is_empty() {
        return None;
    }
    let mut buf = String::from("{ ");
    for info in &s.export_type_infos {
        buf.push_str(info.name.as_str());
        buf.push_str(": ");
        match &info.type_source {
            Some(t) => buf.push_str(t),
            // When no explicit type annotation exists on the local,
            // use `typeof <name>` — a body-scope reference that
            // resolves to whatever TS inferred from the local's
            // initializer. Mirrors upstream svelte2tsx
            // (ExportedNames.ts `createReturnElementsType`): upstream
            // emits `translate?: typeof translate` so a local like
            // `let translate = writable({x:0,y:0})` preserves its
            // `Writable<{x,y}>` type through the default export's
            // Exports slot instead of collapsing to `any`.
            //
            // Critical: the output is embedded INSIDE `$$render`'s
            // body via `return { ... exports: undefined as any as
            // <string> };`, so `typeof <name>` resolves against the
            // body-local declaration. At module scope the same
            // reference would fire TS2304, so any module-scope use
            // of the Exports type MUST go through the
            // `Awaited<ReturnType<typeof $$render>>['exports']`
            // projection instead of the raw string.
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

/// Build the body of a JSDoc `@typedef <body> $$ComponentProps` from a
/// `$props()` destructure. Returns `Some("{name: any, opt?: string}")`
/// (a complete object-type typespec including the outer `{}`) when
/// there's at least one non-rest entry; `None` when the destructure
/// list is empty (caller falls back to `any`).
///
/// Each entry maps to:
///   - `key: any` for required (no default, not $bindable, not rest)
///   - `key?: <inferred>` for optional, where `<inferred>` is the
///     literal-type derived from the default expression (string for
///     `= ''`, `Function` for `= () => {}`, `Record<string, any>`
///     for `= {}`, etc.); falls back to `any` for unrecognised
///     default expressions.
///   - `[key: string]: any` for `...rest` (loosens the typedef so
///     extra props at consumers don't trigger excess-prop errors)
///
/// Mirrors upstream svelte2tsx's `getTypeForDefault` so consumers'
/// callback / scalar mismatches surface the same TS2322 / TS2353
/// diagnostics. See `infer_default_type` in `crates/analyze/src/props.rs`.
pub(crate) fn synthesise_js_props_typedef_body(
    props_info: &svn_analyze::PropsInfo,
) -> Option<String> {
    if props_info.destructures.is_empty() {
        return None;
    }
    let mut body = String::from("{");
    let mut first = true;
    let mut emitted_index_signature = false;
    for entry in &props_info.destructures {
        if entry.is_rest {
            // `...rest` — widen the type with an index signature so any
            // remaining prop the parent passes is accepted. Only one
            // index signature is allowed per type literal.
            if !emitted_index_signature {
                if !first {
                    body.push_str(", ");
                }
                body.push_str("[key: string]: any");
                emitted_index_signature = true;
                first = false;
            }
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
            // Pad newlines INSIDE the ignore-marker span — diagnostics
            // on those lines are dropped by the mapper, but the
            // line-count parity restores correct positions downstream.
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
}

fn annotation_action(declarator: &VariableDeclarator<'_>) -> Option<AnnotationAction> {
    let BindingPattern::ObjectPattern(obj) = &declarator.id else {
        return None;
    };
    // Initializer must be a bare `$props()` call with NO explicit
    // type argument. When the user wrote `$props<T>()` they already
    // expressed the intended type — upstream's `ExportedNames` swaps
    // the generic argument in place with `$$ComponentProps` (via
    // ignore markers) rather than adding a destructure annotation,
    // so we leave it alone on that shape to match. Annotating on top
    // of `$props<T>()` would double-specify and silence downstream
    // errors that upstream catches.
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
    if call.type_arguments.is_some() {
        return None;
    }
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
) {
    if slot_defs.is_empty() {
        out.push_str("undefined as any as {}");
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
        for (name, expr) in &def.attrs {
            if !first_attr {
                out.push_str(", ");
            }
            first_attr = false;
            out.push_str(name.as_str());
            out.push_str(": (");
            match expr {
                // Range form — slice the original source. A `get`
                // miss means the caller passed a source the walker
                // didn't see; fall through with empty parens to
                // preserve the output shape.
                svn_analyze::SlotAttrExpr::Range(range) => {
                    if let Some(text) =
                        source.get(range.start as usize..range.end as usize)
                    {
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
            }
            out.push(')');
        }
        out.push_str(" }");
    }
    out.push_str(" }");
}
