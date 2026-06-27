//! `bind:` directive analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/Binding.ts`.

use svn_parser::{Attribute, Directive, DirectiveValue};

use crate::nodes::attribute::literal_attr_value;
use crate::nodes::destructure::simple_identifier_in;
use crate::walker::{BindThisTarget, Counters, TemplateSummary};

/// Dispatch the target type for a `bind:value` directive based on the
/// element tag + literal `type="..."` sibling attribute. Called by the
/// emit side (`emit::nodes::binding::emit_element_bind_checks_inline`)
/// which re-derives the bind checks inline from the element attributes.
///
/// Returns `None` for tags / type-attr combinations we don't model:
/// - `<input type="file" | "checkbox" | "radio">`: handled by
///   `bind:files` / `bind:checked` (different table entries).
/// - `<select>`: target type depends on `<option>` values; not
///   statically resolvable without option inspection.
/// - Other tags: `bind:value` isn't meaningful.
pub fn resolve_bind_value_type(
    tag_name: &str,
    attrs: &[Attribute],
    source: &str,
) -> Option<&'static str> {
    match tag_name {
        "input" => match literal_attr_value(attrs, "type", source) {
            Some("number") | Some("range") => Some("number"),
            Some("file") | Some("checkbox") | Some("radio") => None,
            _ => Some("string"),
        },
        "textarea" => Some("string"),
        _ => None,
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
                summary.bind_this_targets.push(BindThisTarget { name });
            }
        }
        None => {
            // Bare `bind:foo` is shorthand for `bind:foo={foo}` —
            // same definite-assignment story as the explicit form.
            summary.bind_this_targets.push(BindThisTarget {
                name: d.name.clone(),
            });
        }
        _ => {}
    }
}
