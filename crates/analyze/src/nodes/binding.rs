//! `bind:` directive analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/Binding.ts`.

use svn_parser::{Attribute, Directive, DirectiveValue};

use crate::nodes::attribute::literal_attr_value;
use crate::nodes::destructure::simple_identifier_in;
use crate::walker::{
    BindThisCheck, BindThisTarget, Counters, DomBinding, DomBindingExpression, TemplateSummary,
};

/// v0.3 Item 8 extended: record `bind:value={EXPR}` sites with a
/// context-aware target type resolved from the element tag + literal
/// `type="..."` sibling attribute. Pushes a `DomBinding` entry so the
/// existing Item 6 emit path handles the assignment-direction check
/// + source-map post-scan uniformly.
///
/// Dispatch matrix:
/// - `<input type="number">` / `<input type="range">` → `number`
/// - `<input type="file">`   → SKIP (`bind:files` is the typed path)
/// - `<input>` any other / no type attribute → `string`
/// - `<textarea>` → `string`
/// - other tags (including `<select>`) → SKIP (upstream dispatches
///   via `svelteHTML.createElement` ambient typing; we don't have
///   that wired in, so staying silent matches upstream's "pass"
///   behavior at typecheck level on these cases).
///
/// `bind:group` is intentionally NOT recorded — upstream widens the
/// target to `any` (`__sveltets_2_any(null)`), we simply skip the
/// check entirely which has the same observable no-error outcome.
pub(crate) fn collect_bind_value_bindings(
    attrs: &[Attribute],
    tag_name: &str,
    summary: &mut TemplateSummary,
) {
    let Some(ty) = resolve_bind_value_type(tag_name, attrs) else {
        return;
    };
    for attr in attrs {
        let Attribute::Directive(d) = attr else {
            continue;
        };
        if d.kind != svn_parser::DirectiveKind::Bind || d.name.as_str() != "value" {
            continue;
        }
        let expression = match &d.value {
            Some(svn_parser::DirectiveValue::Expression {
                expression_range, ..
            }) => DomBindingExpression::Range(*expression_range),
            None => DomBindingExpression::Identifier(d.name.clone()),
            _ => continue,
        };
        summary.dom_bindings.push(DomBinding {
            expression,
            type_annotation: ty,
        });
    }
}

/// Dispatch the target type for a `bind:value` directive based on the
/// element tag + literal `type="..."` sibling attribute. Shared by
/// analyze (collection into `summary.dom_bindings`) and emit
/// (inline contract-check generation) so both pipelines stay in sync.
///
/// Returns `None` for tags / type-attr combinations we don't model:
/// - `<input type="file" | "checkbox" | "radio">`: handled by
///   `bind:files` / `bind:checked` (different table entries).
/// - `<select>`: target type depends on `<option>` values; not
///   statically resolvable without option inspection.
/// - Other tags: `bind:value` isn't meaningful.
pub fn resolve_bind_value_type(tag_name: &str, attrs: &[Attribute]) -> Option<&'static str> {
    match tag_name {
        "input" => match literal_attr_value(attrs, "type") {
            Some("number") | Some("range") => Some("number"),
            Some("file") | Some("checkbox") | Some("radio") => None,
            _ => Some("string"),
        },
        "textarea" => Some("string"),
        _ => None,
    }
}

/// v0.3 Item 7: record `bind:this={EXPR}` sites on DOM elements and
/// `<svelte:element>` for emit's source-map post-scan. Emit pairs
/// each entry with a `__svn_bind_this_check<TAG>(EXPR);` overlay
/// occurrence and pushes a TokenMapEntry. Walk order matches emit
/// order; pairing is N-th to N-th.
pub(crate) fn collect_bind_this_checks(attrs: &[Attribute], summary: &mut TemplateSummary) {
    for attr in attrs {
        let Attribute::Directive(d) = attr else {
            continue;
        };
        if d.kind != svn_parser::DirectiveKind::Bind || d.name.as_str() != "this" {
            continue;
        }
        let Some(svn_parser::DirectiveValue::Expression {
            expression_range, ..
        }) = &d.value
        else {
            continue;
        };
        summary.bind_this_checks.push(BindThisCheck {
            expression_range: *expression_range,
        });
    }
}

/// Handle the `bind:` arm of `walk_directive`. Three sub-cases:
///
/// - `BindPair` (Svelte 5 `bind:foo={getter, setter}`) → register
///   a `__svn_bind_pair_N` void-ref.
/// - `Expression` (`bind:foo={x}` / `bind:this={x}`) → record `x`
///   as a definite-assignment target if it's a simple identifier,
///   AND record a DOM-binding type-check entry if `foo` is in our
///   one-way DOM-binding table (contentRect, contentBoxSize, etc.).
/// - `None` (bare `bind:foo`) → desugars to `bind:foo={foo}`; same
///   definite-assignment + DOM-binding treatment as the explicit
///   form, with the identifier source taken from the directive's
///   own range.
pub(crate) fn handle_bind_directive(
    d: &Directive,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    source: &str,
) {
    match &d.value {
        Some(DirectiveValue::BindPair { .. }) => {
            let name = format!("__svn_bind_pair_{}", counters.bind_pair);
            summary.void_refs.register(name);
            counters.bind_pair += 1;
        }
        Some(DirectiveValue::Expression {
            expression_range, ..
        }) => {
            // `bind:this={x}` and `bind:foo={x}` (any prop name) — if
            // the bound value is a simple identifier, that local
            // gets assigned asynchronously by Svelte (bind:this when
            // the element mounts; bind:foo when the child component
            // updates the bound prop). Record it for the definite-
            // assignment rewrite so closures reading the variable
            // don't fire TS2454.
            if let Some(name) = simple_identifier_in(source, *expression_range) {
                summary.bind_this_targets.push(BindThisTarget {
                    name,
                    range: *expression_range,
                });
            }
            // If the binding name is in our DOM-binding type
            // table (contentRect, contentBoxSize, buffered, …),
            // record the value range + its target type so the
            // emit can generate `<x> = __svn_any() as <TYPE>;`
            // in the template-check body. Catches shapes like
            // `<div bind:contentRect={rect}>` where `rect`'s
            // declared type doesn't accept DOMRectReadOnly.
            //
            // This runs IN ADDITION to the bind-target record
            // above — the same variable needs BOTH the
            // definite-assignment `!` rewrite (assignment is
            // hidden inside a lifecycle callback, flow analysis
            // can't see it) AND the type-compatibility check.
            if let Some(type_annotation) = crate::dom_binding::type_for(d.name.as_str()) {
                summary.dom_bindings.push(DomBinding {
                    expression: DomBindingExpression::Range(*expression_range),
                    type_annotation,
                });
            }
        }
        None => {
            // Bare `bind:foo` is shorthand for `bind:foo={foo}` —
            // same definite-assignment story as the explicit form.
            summary.bind_this_targets.push(BindThisTarget {
                name: d.name.clone(),
                range: d.range,
            });
            // Also thread through the DOM-binding type check for
            // bare shorthands like `<video bind:buffered>` which
            // desugar to `bind:buffered={buffered}`.
            if let Some(type_annotation) = crate::dom_binding::type_for(d.name.as_str()) {
                summary.dom_bindings.push(DomBinding {
                    expression: DomBindingExpression::Identifier(d.name.clone()),
                    type_annotation,
                });
            }
        }
        _ => {}
    }
}
