//! Element-attached directive emission: `use:ACTION={PARAMS}`,
//! `bind:NAME={EXPR}`, and the legacy `__svn_action_attrs_N` compat
//! shims kept alive for void-block consumers.
//!
//! `class:` and `style:` directives live in `nodes::element` since
//! they're emitted as bare statements inside the same scoped block as
//! the element's `createElement` call. The directives here all need
//! their own dedicated emit sites — either before `createElement`
//! (action `const` decls), after it (void refs, bind checks), or at
//! `__svn_tpl_check`'s top (legacy attr shims).

use std::fmt::Write;

use svn_analyze::TemplateSummary;

use crate::emit_buffer::EmitBuffer;
use crate::emit_is_ts;
use crate::nodes::element::element_type_annotation;

/// Register the legacy `let __svn_action_attrs_N: any = {};` compat
/// shims at the top of `__svn_tpl_check`. These are registered names
/// consumed by downstream void-ref emission. The actual `use:` action
/// calls that type-check params against action signatures are emitted
/// inline per-element by [`emit_dom_action_decls`] — so scope-bound
/// identifiers referenced inside the params expression
/// (`{#each items as item, i}`'s `i`, `{@const}` declarations, snippet
/// parameters) stay visible in the emitted TS.
pub(crate) fn emit_legacy_action_attrs(out: &mut String, summary: &TemplateSummary, is_ts: bool) {
    // Keep the old `__svn_action_attrs_N` void registration alive so
    // external references (e.g. template spreads that consumed the
    // action's declared `$$_attributes`) still resolve. The outer
    // void_block emits `void __svn_action_attrs_N;` unconditionally;
    // declaring the binding here avoids TS2304 at that reference site.
    for name in summary.void_refs.names() {
        if name.starts_with("__svn_action_attrs_") {
            if is_ts {
                let _ = writeln!(out, "        let {name}: any = {{}};");
            } else {
                let _ = writeln!(out, "        /** @type {{any}} */ let {name} = {{}};");
            }
            let _ = writeln!(out, "        void {name};");
        }
    }
}

/// Emit `const __svn_action_N = __svn_ensure_action(...)` declarations
/// for each `use:` directive on an element. Runs BEFORE the
/// `svelteHTML.createElement` call so the action-attribute returns can
/// be passed as the second arg (3-arg overload), which forces tsgo to
/// expand the attrs param type's alias into `Omit<HTMLAttributes<...>,
/// never> & <actions>` — matching upstream's byte-for-byte diagnostic
/// message format.
///
/// Shape (mirrors upstream svelte2tsx's `__sveltets_2_ensureAction(…)`
/// with our `__svn_` namespace):
///
/// ```ts
///     const __svn_action_0 = __svn_ensure_action(
///         enhance(__svn_map_element_tag('form'), (({formData}) => {…}))
///     );
/// ```
///
/// The inner `enhance(…)` is a real function call — TS checks that the
/// PARAMS match ACTION's declared parameter shape. For `use:enhance=
/// {({form,data,submit}) => …}` that fires TS2339 on each wrong name
/// because the real `SubmitFunction` parameter type doesn't have them.
///
/// Returns the range of indices allocated (`first..first+count`) so the
/// caller can splice the union at the createElement site and emit
/// matching `void __svn_action_N;` references after the element closes.
///
/// Inline emission at element depth puts each call INSIDE any enclosing
/// `{#each} as item, index` / `{@const X = …}` / `{#snippet}` scope, so
/// identifiers defined there resolve correctly when referenced from the
/// callback bodies inside PARAMS.
pub(crate) fn emit_dom_action_decls(
    buf: &mut EmitBuffer,
    source: &str,
    tag_name: &str,
    attributes: &[svn_parser::Attribute],
    depth: usize,
    action_counter: &mut usize,
) -> std::ops::Range<usize> {
    let indent = "    ".repeat(depth);
    let tag_arg = if tag_name.is_empty() {
        "'' as string".to_string()
    } else {
        format!("'{tag_name}'")
    };
    let first = *action_counter;
    for attr in attributes {
        let svn_parser::Attribute::Directive(d) = attr else {
            continue;
        };
        if d.kind != svn_parser::DirectiveKind::Use {
            continue;
        }
        let index = *action_counter;
        *action_counter += 1;
        let action = d.name.as_str();
        match &d.value {
            Some(svn_parser::DirectiveValue::Expression {
                expression_range, ..
            }) => {
                let Some(params) =
                    source.get(expression_range.start as usize..expression_range.end as usize)
                else {
                    // Undo the counter bump on skip so subsequent
                    // indices stay contiguous.
                    *action_counter -= 1;
                    continue;
                };
                // `params` splices via `append_with_source` so the
                // TokenMapEntry covering the params overlay span maps
                // back to the user's `use:ACTION={...}` expression_range
                // at emit time.
                let _ = write!(
                    buf,
                    "{indent}const __svn_action_{index} = __svn_ensure_action({action}(__svn_map_element_tag({tag_arg}), ("
                );
                buf.append_with_source(params, *expression_range);
                buf.push_str(")));\n");
            }
            _ => {
                let _ = writeln!(
                    buf,
                    "{indent}const __svn_action_{index} = __svn_ensure_action({action}(__svn_map_element_tag({tag_arg})));"
                );
            }
        }
    }
    first..*action_counter
}

/// Emit `void __svn_action_N;` references for each allocated action
/// index. Placed AFTER the `svelteHTML.createElement` call to prevent
/// the unused-variable diagnostic — the action's value is carried into
/// `createElement`'s second arg, then referenced here to mark it used.
pub(crate) fn emit_dom_action_void_refs(
    buf: &mut EmitBuffer,
    indices: &std::ops::Range<usize>,
    depth: usize,
) {
    let indent = "    ".repeat(depth);
    for index in indices.clone() {
        let _ = writeln!(buf, "{indent}void __svn_action_{index};");
    }
}

/// Legacy emission for the feature-gated non-dom-emit path: emit both
/// the action declaration and its void-reference in one block, without
/// feeding the action into a createElement 3-arg overload. Preserves
/// the pre-Phase-2 behavior when `SVN_DOM_ELEMENT_EMIT=0` opts out of
/// the DOM-element emit entirely.
pub(crate) fn emit_use_directives_inline_legacy(
    buf: &mut EmitBuffer,
    source: &str,
    tag_name: &str,
    attributes: &[svn_parser::Attribute],
    depth: usize,
    action_counter: &mut usize,
) {
    let indices = emit_dom_action_decls(buf, source, tag_name, attributes, depth, action_counter);
    emit_dom_action_void_refs(buf, &indices, depth);
}

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
