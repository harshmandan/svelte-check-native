//! DOM element + `<svelte:*>` element emission (the dispatch layer).
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Element.ts`.
//!
//! Each `<tag …>` becomes a `svelteHTML.createElement("tag", { …attrs })`
//! call wrapped in a scoped `{ }` block. Per-attribute-type emit lives
//! in sibling modules:
//!
//! - [`crate::nodes::attribute`] — plain / expression / shorthand
//!   attribute entries + the attribute-skip table.
//! - [`crate::nodes::spread`] — `{...EXPR}` spreads.
//! - [`crate::nodes::attach_tag`] — `{@attach EXPR}` attachments
//!   (parser routes these through the spread shape with `is_attach:
//!   true`).
//! - [`crate::nodes::class`] — `class:foo={…}` directive checks.
//! - [`crate::nodes::style_directive`] — `style:foo={…}` directive
//!   checks.
//! - [`crate::nodes::animation`] — `animate:NAME(…)` directive call
//!   typing.
//! - [`crate::nodes::transition`] — `transition:` / `in:` / `out:`
//!   directive call typing.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;
use crate::nodes::animation::emit_animation_directive;
use crate::nodes::attribute::{emit_expression, emit_plain, emit_shorthand, should_skip};
use crate::nodes::class::emit_class_directive;
use crate::nodes::spread::emit_spread;
use crate::nodes::style_directive::emit_style_directive;
use crate::nodes::transition::emit_transition_directive;

/// Emit the upstream-shape `svelteHTML.createElement("tag", { …attrs });`
/// call for a DOM element. Opens a scoped `{ }` block so element-local
/// let-bindings (`{@const}`, `let:x`, action-attr `const $$action_N`)
/// stay confined to this element — matches upstream Element.ts's
/// transformation result.
///
/// Output shape (matches upstream svelte2tsx):
///   `{ svelteHTML.createElement("tag", { "name": value, name2, … }); `
/// Closing `}` is emitted by the caller after children + bind/use
/// checks recurse into the same scope. `tag_literal` controls whether
/// the first arg is a quoted string literal (`"div"`) — set false for
/// `svelte:element this={tag}` where the caller passes the expression
/// verbatim as `tag_name`.
pub(crate) fn emit_dom_element_open(
    buf: &mut EmitBuffer,
    source: &str,
    tag_name: &str,
    tag_literal: bool,
    attributes: &[svn_parser::Attribute],
    depth: usize,
    action_indices: &std::ops::Range<usize>,
) {
    let indent = "    ".repeat(depth);
    // Build the `__svn_union(__svn_action_0, __svn_action_1, …)`
    // second arg when any `use:` directives are present. Matches
    // upstream `svelte2tsx`'s 3-arg overload emit — the `attrs`
    // parameter type becomes `Elements[Key] & T` (intersection)
    // which tsgo eagerly expands in error messages.
    let union_prefix = if action_indices.is_empty() {
        String::new()
    } else {
        let mut args = String::new();
        for (i, index) in action_indices.clone().enumerate() {
            if i > 0 {
                args.push_str(", ");
            }
            let _ = write!(args, "__svn_action_{index}");
        }
        format!("__svn_union({args}), ")
    };
    if tag_literal {
        let _ = write!(
            buf,
            "{indent}{{ svelteHTML.createElement(\"{tag_name}\", {union_prefix}{{"
        );
    } else {
        let _ = write!(
            buf,
            "{indent}{{ svelteHTML.createElement({tag_name}, {union_prefix}{{"
        );
    }
    let mut any = false;
    for attr in attributes {
        match attr {
            svn_parser::Attribute::Plain(p) => {
                if should_skip(p.name.as_str()) {
                    continue;
                }
                if !any {
                    buf.push_str("\n");
                    any = true;
                }
                emit_plain(buf, source, p, depth + 1);
            }
            svn_parser::Attribute::Expression(e) => {
                if should_skip(e.name.as_str()) {
                    continue;
                }
                if !any {
                    buf.push_str("\n");
                    any = true;
                }
                emit_expression(buf, source, e, depth + 1);
            }
            svn_parser::Attribute::Shorthand(s) => {
                if should_skip(s.name.as_str()) {
                    continue;
                }
                if !any {
                    buf.push_str("\n");
                    any = true;
                }
                emit_shorthand(buf, source, s, depth + 1);
            }
            svn_parser::Attribute::Spread(s) => {
                // Bail-check first (skip empty / whitespace-only
                // expressions) so the leading newline only flushes
                // when we're going to emit. Mirrors the per-attribute
                // pattern used for plain/expression/shorthand above.
                if !crate::nodes::spread::can_emit(source, s) {
                    continue;
                }
                if !any {
                    buf.push_str("\n");
                    any = true;
                }
                emit_spread(buf, source, s, depth);
            }
            // Directives (bind:, use:, class:, style:, transition:, …)
            // are handled outside createElement — by the bind/use
            // passes and by emit_dom_directive_checks.
            svn_parser::Attribute::Directive(_) => {}
        }
    }
    if any {
        let _ = writeln!(buf, "{indent}}});");
    } else {
        buf.push_str("}); ");
    }
}

/// Post-createElement directive checks for `class:`, `style:`,
/// `animate:`, `transition:` / `in:` / `out:` attributes. Emit each
/// directive's check as a bare statement inside the element's scoped
/// block. Per-directive-kind emit lives in [`crate::nodes::class`],
/// [`crate::nodes::style_directive`], [`crate::nodes::animation`],
/// and [`crate::nodes::transition`].
///
/// `tag_name` threads through to animation/transition handlers — they
/// emit `__svn_map_element_tag('TAG')` so the directive function gets
/// a typed `HTMLElementTagNameMap[TAG]` first argument (ties the
/// directive's element-type generic to the host tag).
pub(crate) fn emit_dom_directive_checks(
    buf: &mut EmitBuffer,
    source: &str,
    tag_name: &str,
    attributes: &[svn_parser::Attribute],
    depth: usize,
) {
    let indent = "    ".repeat(depth);
    for attr in attributes {
        let svn_parser::Attribute::Directive(d) = attr else {
            continue;
        };
        match d.kind {
            svn_parser::DirectiveKind::Class => emit_class_directive(buf, source, d, &indent),
            svn_parser::DirectiveKind::Style => emit_style_directive(buf, source, d, &indent),
            svn_parser::DirectiveKind::Animate => {
                emit_animation_directive(buf, source, d, &indent, tag_name)
            }
            svn_parser::DirectiveKind::Transition
            | svn_parser::DirectiveKind::In
            | svn_parser::DirectiveKind::Out => {
                emit_transition_directive(buf, source, d, &indent, tag_name)
            }
            _ => {}
        }
    }
}

/// Close the scoped block opened by `emit_dom_element_open`.
pub(crate) fn emit_dom_element_close(buf: &mut EmitBuffer, depth: usize) {
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}}}");
}

/// Open a `svelteHTML.createElement` scoped block for a
/// `<svelte:*>` element. Dispatches on `SvelteElementKind`:
///   - Body/Head/Window/Document/Options/Fragment: literal
///     `"svelte:<name>"` tag string. IntrinsicElements in our
///     shim has these as named keys.
///   - Element: `<svelte:element this={expr}>` — pass the `this`
///     expression verbatim as the first createElement arg so TS
///     checks the tag against IntrinsicElements keys.
///   - SelfRef/Component/Boundary/missing-this: skip the
///     createElement scope (not DOM elements). Open a bare
///     `{ }` block so child emit still scopes locals correctly.
pub(crate) fn emit_svelte_element_open(
    buf: &mut EmitBuffer,
    source: &str,
    s: &svn_parser::SvelteElement,
    depth: usize,
    action_indices: &std::ops::Range<usize>,
) {
    use svn_parser::SvelteElementKind::*;
    let indent = "    ".repeat(depth);
    match s.kind {
        Body | Head | Window | Document | Options | Fragment => {
            let tag = format!("svelte:{}", s.kind.as_str());
            emit_dom_element_open(
                buf,
                source,
                &tag,
                true,
                &s.attributes,
                depth,
                action_indices,
            );
        }
        Element => {
            // Find `this={expr}` among attributes.
            let this_expr = s.attributes.iter().find_map(|a| {
                let svn_parser::Attribute::Expression(e) = a else {
                    return None;
                };
                if e.name.as_str() != "this" {
                    return None;
                }
                source
                    .get(e.expression_range.start as usize..e.expression_range.end as usize)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            });
            match this_expr {
                Some(expr) => {
                    emit_dom_element_open(
                        buf,
                        source,
                        &format!("({expr})"),
                        false,
                        &s.attributes,
                        depth,
                        action_indices,
                    );
                }
                None => {
                    // Missing `this` — bare scope. Child emit still runs.
                    let _ = writeln!(buf, "{indent}{{");
                }
            }
        }
        SelfRef | Component | Boundary => {
            // Not a DOM element — bare scope for children.
            let _ = writeln!(buf, "{indent}{{");
        }
    }
}

/// Map a static HTML/SVG tag name to a `HTMLElementTagNameMap['tag']`
/// / `SVGElementTagNameMap['tag']` indexed-access type. Dynamic
/// tags (empty string) fall back to `HTMLElement`. Unknown tag
/// names that aren't in either map would resolve through these
/// indexed accesses to `any` — acceptable, just means the check
/// stays lax for custom elements.
pub(crate) fn element_type_annotation(tag_name: &str) -> String {
    if tag_name.is_empty() {
        return "HTMLElement".to_string();
    }
    format!("HTMLElementTagNameMap['{tag_name}']")
}
