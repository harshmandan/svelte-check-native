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
/// (NOT wrapped in a never-called lambda).
///
/// For upstream's ONE-WAY bindings (`bind:this`, `clientWidth`,
/// `naturalHeight`, the ResizeObserver/media-list family) this matches
/// upstream svelte2tsx's `appendOneWayBinding` direct form
/// (`Binding.ts:86-136`), which checks TYPE assignable to EXPR.
///
/// For TWO-WAY bindings (`bind:value`, `bind:checked`, `bind:files`)
/// this is a DELIBERATE deviation: upstream emits a widening lambda
/// `() => EXPR = __sveltets_2_any(null);` (`Binding.ts:139-146`) that
/// does NOT check the value, and runs the real check in the opposite
/// direction via `addAttribute` (binding value assignable to the
/// attribute type). Our direct form additionally narrows EXPR's flow
/// type for subsequent uses — e.g. `let x = $state<number>()`
/// (`number | undefined`) flows as `number` after `bind:clientWidth`
/// at a later `<Child {x}/>` site. An earlier lambda wrapper on our
/// side isolated the assignment from narrowing, producing spurious
/// "possibly undefined" errors.
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
    let bind_value_type = svn_analyze::resolve_bind_value_type(tag_name, attributes, source);
    for attr in attributes {
        let svn_parser::Attribute::Directive(directive) = attr else {
            continue;
        };
        if directive.kind != svn_parser::DirectiveKind::Bind {
            continue;
        }
        let name = directive.name.as_str();
        // `bind:group` on `<input>` — upstream Binding.ts:99-109 emits a
        // widening reassignment `EXPR = __sveltets_2_any(null);`. It (a)
        // references the bound variable (so `bind:group={typo}` fires
        // TS2304) and (b) resets flow-narrowing of a single-write
        // variable. `type_for("group")` is None, so this must run BEFORE
        // the type dispatch below or the directive is dropped. The LHS is
        // source-mapped so TS2304 lands on the user's expression; the RHS
        // is `any` so no spurious TS2322. `$store`-valued targets work
        // because the `let $store` subscribe decl is emitted at render-fn
        // scope (lib.rs:1243), dominating this bind site.
        if name == "group" && tag_name == "input" {
            let (expr_text, expr_range): (std::borrow::Cow<'_, str>, Option<svn_core::Range>) =
                match &directive.value {
                    Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) => {
                        let Some(slice) = source
                            .get(expression_range.start as usize..expression_range.end as usize)
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
                    // Shorthand `bind:group` == `bind:group={group}`.
                    None => (std::borrow::Cow::Borrowed(directive.name.as_str()), None),
                    // get/set form: upstream handles group only in the
                    // non-get-set branch — drop (matches prior behaviour).
                    _ => continue,
                };
            buf.push_str(&indent);
            match expr_range {
                Some(r) => buf.append_with_source(&expr_text, r),
                None => buf.push_str(&expr_text),
            }
            buf.push_str(" = __svn_any(null);\n");
            continue;
        }
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
        // Two-way bindings (`bind:checked` / `bind:files`): upstream
        // (Binding.ts:139-201) checks the bound value AGAINST the slot
        // type (value→slot) and emits a widening `() => EXPR = any` lambda.
        // We reproduce both diagnostics:
        //   1. A tuple-element check `const __svn_t: [<slot>] = [EXPR];`
        //      fires TS2322 at the user's expression on a type mismatch —
        //      same code, direction, and position as upstream's
        //      createElement-property check. A tuple ELEMENT mismatch
        //      reports at the element value (unlike an object-literal
        //      property, which reports at the key, or an assignment, which
        //      reports at the LHS — both synth positions that get dropped).
        //      The nullable slot type is hardcoded so this works without
        //      `svelte/elements` installed.
        //   2. The ignore-wrapped, never-called widen lambda widens EXPR's
        //      declared type. This replaces the one-way families'
        //      slot→value assignment, which was laxer on widened-union
        //      targets.
        if let Some(slot) = svn_analyze::dom_binding::two_way_slot_type(name) {
            buf.push_str(&indent);
            if emit_is_ts() {
                let _ = write!(buf, "{{ const __svn_t: [{slot}] = [");
            } else {
                let _ = write!(buf, "{{ /** @type {{[{slot}]}} */ const __svn_t = [");
            }
            match expr_source_range {
                Some(range) => buf.append_with_source(&expr_text, range),
                None => buf.push_str(&expr_text),
            }
            buf.push_str("]; void __svn_t; }\n");
            buf.push_str(&indent);
            buf.push_str("/*svn:ignore_start*/void (() => { ");
            buf.push_str(&expr_text);
            buf.push_str(" = __svn_any(null); });/*svn:ignore_end*/\n");
            continue;
        }
        buf.push_str(&indent);
        match expr_source_range {
            Some(range) => buf.append_with_source(&expr_text, range),
            None => buf.push_str(&expr_text),
        }
        // R-Conv #20: for `<svelte:element this={tagExpr} bind:this={target}>`
        // narrow the RHS through `svelteHTML.createElement(tagExpr, {})` so
        // when `tagExpr` is a literal type (e.g. `'div'`), TS resolves the
        // return to `HTMLDivElement` instead of the loose `HTMLElement`
        // annotation. Mirrors upstream svelte2tsx's
        // `const $$_svelteelement0 = svelteHTML.createElement(tag, {…});
        // target = $$_svelteelement0;` shape (Element.ts).
        let svelte_element_this_expr = (tag_name.is_empty() && name == "this")
            .then(|| svelte_element_tag_expr(attributes, source))
            .flatten();
        // Element-native one-way bindings (clientWidth, naturalWidth, …)
        // on a static tag resolve through the tag's createElement return,
        // mirroring upstream's `EXPR = element.NAME` (Binding.ts:112-115):
        // a binding on the wrong element (`bind:naturalWidth` on `<div>`)
        // fires TS2339, and unknown/custom tags fall back to `any` via
        // the createElement overload. Valid in both JS and TS overlays.
        if svn_analyze::dom_binding::is_element_native_oneway(name) && !tag_name.is_empty() {
            // Bind the element to a concrete-typed local first, then read
            // the property off it. A concrete local (vs an inline
            // `createElement(...).NAME`) stops TS's generic inference from
            // escaping to the `any`-tag overload, so a binding on the
            // WRONG element (`bind:naturalWidth` on `<div>`) fires TS2339
            // like upstream's `EXPR = element.NAME` (Binding.ts:112-115).
            // The property name is source-mapped to the `bind:NAME` span
            // (`bind:` is 5 bytes) so that TS2339 surfaces at the user's
            // directive rather than being dropped as unmapped synth code.
            // `EXPR` was already appended above; complete the assignment.
            let name_range = svn_core::Range::new(
                directive.range.start + 5,
                directive.range.start + 5 + name.len() as u32,
            );
            buf.push_str(" = (() => { const __svn_el = svelteHTML.createElement(\"");
            buf.push_str(tag_name);
            buf.push_str("\", {}); return __svn_el.");
            buf.append_with_source(name, name_range);
            buf.push_str("; })();\n");
            continue;
        }
        if emit_is_ts() {
            if let Some((tag_expr, tag_range)) = &svelte_element_this_expr {
                buf.push_str(" = svelteHTML.createElement(");
                buf.append_with_source(tag_expr, *tag_range);
                buf.push_str(", {});\n");
            } else {
                let _ = writeln!(buf, " = null as any as {ty};");
            }
        } else if let Some((tag_expr, tag_range)) = &svelte_element_this_expr {
            buf.push_str(" = svelteHTML.createElement(");
            buf.append_with_source(tag_expr, *tag_range);
            buf.push_str(", {});\n");
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

/// Extract the `this={EXPR}` expression text + source range from a
/// `<svelte:element>`'s attribute list. Used to narrow the
/// `bind:this={target}` RHS through `svelteHTML.createElement(EXPR,
/// {})` so a literal-typed `tag: 'div'` produces `HTMLDivElement`
/// (not the loose `HTMLElement` fallback).
fn svelte_element_tag_expr<'a>(
    attributes: &[svn_parser::Attribute],
    source: &'a str,
) -> Option<(std::borrow::Cow<'a, str>, svn_core::Range)> {
    for attr in attributes {
        let svn_parser::Attribute::Expression(e) = attr else {
            continue;
        };
        if e.name.as_str() != "this" {
            continue;
        }
        let slice =
            source.get(e.expression_range.start as usize..e.expression_range.end as usize)?;
        let trimmed = slice.trim();
        if trimmed.is_empty() {
            return None;
        }
        let leading_ws = (slice.len() - slice.trim_start().len()) as u32;
        let start = e.expression_range.start + leading_ws;
        let end = start + trimmed.len() as u32;
        return Some((
            std::borrow::Cow::Borrowed(trimmed),
            svn_core::Range::new(start, end),
        ));
    }
    None
}
