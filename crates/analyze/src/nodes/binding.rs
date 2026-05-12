//! `bind:` directive analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/Binding.ts`.

use svn_parser::Attribute;

use crate::nodes::attribute::literal_attr_value;
use crate::template_walker::{
    BindThisCheck, DomBinding, DomBindingExpression, TemplateSummary,
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
