//! `use:ACTION={PARAMS}` action-directive emission, plus the legacy
//! `__svn_action_attrs_N` compat shims kept alive for void-block
//! consumers.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Action.ts`.

use std::fmt::Write;

use svn_analyze::TemplateSummary;

use crate::emit_buffer::EmitBuffer;

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
        // Source range covering the action name (`enhance` in
        // `use:enhance={params}`). `d.range.start` is the byte
        // offset of the `use:` prefix; the name starts after `use:`
        // (kind str + 1 for the colon). Used to anchor TS2304/TS2552
        // diagnostics on typo'd action names back to the user's
        // source position via the token map. Without this, a typo'd
        // action name lands in synthesized scaffolding (no line_map
        // coverage) and the diagnostic mapper drops the diagnostic.
        let prefix_len = (d.kind.as_str().len() + 1) as u32;
        let name_start = d.range.start + prefix_len;
        let name_end = name_start + action.len() as u32;
        let name_range = svn_core::Range::new(name_start, name_end);
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
                    "{indent}const __svn_action_{index} = __svn_ensure_action("
                );
                buf.append_with_source(action, name_range);
                let _ = write!(buf, "(__svn_map_element_tag({tag_arg}), (",);
                buf.append_with_source(params, *expression_range);
                buf.push_str(")));\n");
            }
            _ => {
                let _ = write!(
                    buf,
                    "{indent}const __svn_action_{index} = __svn_ensure_action("
                );
                buf.append_with_source(action, name_range);
                let _ = writeln!(buf, "(__svn_map_element_tag({tag_arg})));");
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
