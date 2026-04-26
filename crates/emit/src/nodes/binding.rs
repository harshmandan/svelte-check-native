//! `bind:NAME={EXPR}` binding-directive emission for DOM elements.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Binding.ts`.
//!
//! Component bindings (`bind:VALUE` on `<Comp>` instantiations) are
//! emitted via the prop-shape writer in [`crate::nodes::inline_component`]
//! — they thread through the component-call's props literal and the
//! post-`new` widen trailers, not as standalone bind checks.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;
use crate::emit_is_ts;
use crate::nodes::element::element_type_annotation;

/// Emit a type-check line per `bind:NAME` directive on a DOM element.
///
/// Shape: `{indent}EXPR = null as any as TYPE;` — direct assignment
/// (NOT wrapped in a never-called lambda). Upstream svelte2tsx's
/// `Binding.ts:86-146` emits the same direct form, which type-checks
/// the LHS against the binding's value type AND narrows EXPR's flow
/// type for subsequent uses. A `let x = $state<number>()` (type
/// `number | undefined`) passed through `bind:clientWidth` then flows
/// as `number` at the later `<Child {x}/>` site. The earlier lambda
/// wrapper isolated the assignment from narrowing, producing spurious
/// "possibly undefined" errors that upstream didn't.
///
/// Supports all DOM bind: variants under one loop:
///   - `bind:this` — TYPE = `HTMLElementTagNameMap['tag']` (or
///     `HTMLElement` for the dynamic `<svelte:element>` escape hatch
///     when `tag_name == ""`). Member expressions
///     (`bind:this={refs.input}`) and bare identifiers both work;
///     the assignment is verbatim from source.
///   - `bind:value` — TYPE resolved once per element via
///     `svn_analyze::resolve_bind_value_type`, which inspects the
///     literal `type="..."` sibling attribute. Non-form elements
///     return `None` and the directive is skipped.
///   - Other one-way bindings (`bind:checked`, `bind:files`,
///     `bind:group`, `bind:clientWidth`, `bind:naturalHeight`, …) —
///     TYPE from `svn_analyze::dom_binding::type_for(name)`;
///     unknown names are skipped.
///
/// EXPR resolution:
///   - `bind:NAME={expr}` → trimmed EXPR with a source range that
///     exactly covers the trimmed slice; `append_with_source` pushes
///     a TokenMapEntry so diagnostics land at the source position.
///   - `bind:NAME` (shorthand, NAME ≠ `this`) → uses NAME as the
///     target; no source range since there's no user expression to
///     map back to.
///   - `bind:this` without an `={EXPR}` value is not valid Svelte
///     shorthand; skipped.
pub(crate) fn emit_element_bind_checks_inline(
    buf: &mut EmitBuffer,
    source: &str,
    tag_name: &str,
    attributes: &[svn_parser::Attribute],
    depth: usize,
) {
    let indent = "    ".repeat(depth);
    // `bind:value`'s target type depends on the element tag + literal
    // `type="..."` sibling attribute. Resolve once per element since
    // every `bind:value` on the same element dispatches to the same
    // target type.
    let bind_value_type = svn_analyze::resolve_bind_value_type(tag_name, attributes);
    for attr in attributes {
        let svn_parser::Attribute::Directive(directive) = attr else {
            continue;
        };
        if directive.kind != svn_parser::DirectiveKind::Bind {
            continue;
        }
        let name = directive.name.as_str();
        let ty: String = if name == "this" {
            element_type_annotation(tag_name)
        } else if name == "value" {
            match bind_value_type {
                Some(t) => t.to_string(),
                None => continue,
            }
        } else {
            match svn_analyze::dom_binding::type_for(name) {
                Some(t) => t.to_string(),
                None => continue,
            }
        };
        let (expr_text, expr_source_range): (std::borrow::Cow<'_, str>, Option<svn_core::Range>) =
            match &directive.value {
                Some(svn_parser::DirectiveValue::Expression {
                    expression_range, ..
                }) => {
                    let Some(slice) =
                        source.get(expression_range.start as usize..expression_range.end as usize)
                    else {
                        continue;
                    };
                    let trimmed = slice.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let leading_ws = (slice.len() - slice.trim_start().len()) as u32;
                    let start = expression_range.start + leading_ws;
                    let end = start + trimmed.len() as u32;
                    (
                        std::borrow::Cow::Borrowed(trimmed),
                        Some(svn_core::Range::new(start, end)),
                    )
                }
                None => {
                    // `bind:this` has no shorthand form — always
                    // carries `={EXPR}`. Skip when missing.
                    if name == "this" {
                        continue;
                    }
                    (std::borrow::Cow::Borrowed(directive.name.as_str()), None)
                }
                // Svelte 5 `bind:X={get, set}` on DOM — two cases:
                //
                // - `bind:this={get, set}`: mirror upstream's direct
                //   `(set)($$_element)` call. Setter's parameter is
                //   checked by assignability — accepts the element
                //   type or any supertype. Getter ignored.
                //
                // - Other directives (`bind:clientWidth={null, set}`,
                //   `bind:value={get, set}`): route through the
                //   `__svn_get_set_binding` helper with a `satisfies`
                //   trailer so the setter's parameter is unified with
                //   the DOM target's type, exactly like the component
                //   path in `write_prop_shape`. TS1360 fires on
                //   mismatch.
                Some(svn_parser::DirectiveValue::BindPair {
                    getter_range,
                    setter_range,
                    ..
                }) => {
                    let getter = &source[getter_range.start as usize..getter_range.end as usize];
                    let setter = &source[setter_range.start as usize..setter_range.end as usize];
                    buf.push_str(&indent);
                    if name == "this" {
                        buf.push_str("(");
                        buf.append_with_source(setter, *setter_range);
                        let _ = writeln!(buf, ")(null as any as {ty});");
                    } else {
                        buf.push_str("void (__svn_get_set_binding(");
                        buf.append_with_source(getter, *getter_range);
                        buf.push_str(", ");
                        buf.append_with_source(setter, *setter_range);
                        let _ = writeln!(buf, ") satisfies {ty});");
                    }
                    continue;
                }
                _ => continue,
            };
        if expr_text.is_empty() {
            continue;
        }
        buf.push_str(&indent);
        match expr_source_range {
            Some(range) => buf.append_with_source(&expr_text, range),
            None => buf.push_str(&expr_text),
        }
        if emit_is_ts() {
            let _ = writeln!(buf, " = null as any as {ty};");
        } else {
            // JS overlay: `as T` is TS-only syntax. Use a JSDoc cast
            // on the RHS instead — `/** @type {T} */(null)` gives the
            // null literal type T, which assigns into the LHS (the
            // bound variable) and fires TS2322 when the LHS's declared
            // type can't accept T.
            let _ = writeln!(buf, " = /** @type {{{ty}}} */ (null);");
        }
    }
}
