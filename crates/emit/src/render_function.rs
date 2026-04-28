//! `$$render` function emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/createRenderFunction.ts`.
//!
//! Two entry points:
//!
//! - [`emit_template_check_fn`] — the `async function __svn_tpl_check()
//!   { … }` wrapper that carries every template expression as real
//!   TypeScript. The walk produces per-component prop-checks /
//!   bind:this assignments / DOM-binding assignments inline, all pinned
//!   to the enclosing block's scope so block-local refs resolve.
//! - [`emit_render_body_return`] — the trailing `return { props,
//!   events, slots, exports, bindings };` of the `$$render_<hash>()`
//!   wrapping function. The default-export shape extracts each field
//!   via `Awaited<ReturnType<typeof $$render>>['<field>']`.
//!
//! The orchestrator that invokes both lives in `lib.rs`.

use std::fmt::Write;

use crate::emit_bind_pair_declarations;
use crate::emit_buffer::EmitBuffer;
use crate::emit_is_ts;
use crate::emit_template_body;
use crate::nodes::action::emit_legacy_action_attrs;
use crate::props_emit::{synthesise_js_props_typedef_body, write_slots_field_type};
use crate::svelte4::compat::has_strict_events;
use svn_analyze::{TemplateSummary, scan_jsdoc_typedef_name, should_synthesise_js_props};

/// Emit the `async function __svn_tpl_check() { … }` wrapper that
/// carries every template expression as real TypeScript. The walk
/// produces per-component prop-checks / bind:this assignments / DOM-
/// binding assignments inline — all pinned to the enclosing block's
/// scope (`{#each as item, i}`, `{#snippet args}`) so block-local refs
/// resolve correctly.
///
/// Legacy action-attr and bind-pair declarations are emitted BEFORE
/// the walk (both live at the top of the wrapper). They write directly
/// via `raw_string_mut`, so the buffer's line counter needs
/// `resync_current_line()` before the walk starts — any `LineMapEntry`
/// the walk pushes reads the current overlay line from that counter.
pub(crate) fn emit_template_check_fn(
    buf: &mut EmitBuffer,
    doc: &svn_parser::Document<'_>,
    fragment: &svn_parser::Fragment,
    summary: &TemplateSummary,
    is_ts: bool,
) {
    // Arrow expression statement (NOT a function declaration) — TS's
    // control-flow narrowing carries assignment-narrowed types from
    // the enclosing render scope INTO the closure body. A named
    // `async function __svn_tpl_check() {}` declaration is hoisted
    // and TS treats it as if callable before the user's reassignment,
    // which collapses any `let project = ... ; project = X ?? Y;`
    // narrowing back to the declared union type. The arrow-expression
    // form preserves narrowing — see design/gap_c_assignment_narrowing/.
    buf.push_str("    ;(async () => {\n");
    buf.push_str("        // template type-check body (incremental)\n");
    emit_legacy_action_attrs(buf.raw_string_mut(), summary, is_ts);
    emit_bind_pair_declarations(buf.raw_string_mut(), summary, is_ts);
    // Index component instantiations by source byte offset so the
    // template walker can emit each prop-check inline at the component
    // node's position — i.e. inside the enclosing `{#each}` / `{#if}`
    // / `{#snippet}` scope. Flat-block emission put every check at
    // the top level of `__svn_tpl_check`, which silently broke any
    // check whose prop expressions referenced a binding introduced by
    // a block.
    let instantiations_by_start: std::collections::HashMap<
        u32,
        &svn_analyze::ComponentInstantiation,
    > = summary
        .component_instantiations
        .iter()
        .map(|i| (i.node_start, i))
        .collect();
    let mut action_counter: usize = 0;
    buf.resync_current_line();
    emit_template_body(
        buf,
        doc.source,
        fragment,
        2,
        &instantiations_by_start,
        &mut action_counter,
    );
    buf.push_str("    });\n");
}

/// Emit the class-wrapper's `return { props, events, slots, bindings,
/// exports };` at the tail of `$$render_<hash>`'s body. Only fires when
/// both generics and a Props type source are present (the gate
/// `use_class_wrapper`) — a sibling `declare class __svn_Render_<hash>`
/// later projects each of the five surfaces back out at module scope
/// via `Awaited<ReturnType<typeof $$render<…>>>['<field>']`.
///
/// Using `undefined as any as <T>` (not `null as <T>`) so `<T>` can be
/// a non-nullable type like `{ foo: string }` without firing TS2352.
/// Body-local `typeof X` / `$$Props['x']` refs inside `<T>` resolve
/// inside the render function's scope where X / $$Props live.
///
/// `events_field` expands to `$$Events` when user-declared or
/// synthesised under the three-trigger gate; otherwise stays `{}` to
/// preserve lax event handling.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_render_body_return(
    buf: &mut EmitBuffer,
    doc: &svn_parser::Document<'_>,
    generics: Option<&str>,
    prop_type_source: Option<&str>,
    synthesized_events_type: Option<&str>,
    exports_object: Option<&str>,
    props_info: &svn_analyze::PropsInfo,
    slot_defs: &[svn_analyze::SlotDef],
) {
    // JS overlay: always emit a return so the default-export's
    // `Awaited<ReturnType<typeof $$render>>['props']` extraction
    // resolves to a real Props type. When the script has a
    // `/** @typedef {Object} Props */` block and
    // `/** @type {Props} */ let {...} = $props()`, PropsInfo captures
    // the root name ("Props"), and we reference it here via
    // `/** @type {Props} */({})`. Without a Props name, fall back to
    // `any` (degrades to no excess-prop check, but no regression).
    if !emit_is_ts() {
        // Prefer a TS-annotated Props name if PropsInfo captured one,
        // else fall back to scanning the instance script for a JSDoc
        // `@typedef {Object} <Name>` declaration — the standard
        // Svelte-4/JS-Svelte props shape.
        let name_from_ts = prop_type_source.and_then(|ty| {
            let root = ty.trim();
            if !root.is_empty()
                && root
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
                && !root.chars().next().is_some_and(|c| c.is_ascii_digit())
            {
                Some(root.to_string())
            } else {
                None
            }
        });
        let name_from_jsdoc = doc
            .instance_script
            .as_ref()
            .and_then(|s| scan_jsdoc_typedef_name(s.content));
        // Svelte-4 `export let` synthesis: PropsInfo captures a
        // literal `{k: T, …}` type_text (PropsSource::SynthesisedFromExports)
        // that doesn't match the named-type predicate above. Embed the
        // literal body directly as the JSDoc `@type` so the default
        // export's `Awaited<ReturnType<…>>['props']` resolves to the
        // typed shape (e.g. `{b: any}`) — restoring the required-prop
        // signal that powers TS2741 on consumers.
        let literal_from_exports = prop_type_source
            .filter(|_| {
                matches!(
                    props_info.source,
                    svn_analyze::PropsSource::SynthesisedFromExports
                )
            })
            .map(|ty| ty.trim().to_string());
        // Selection precedence — mirrors the synthesis decision in
        // emit_render (must match or `$$ComponentProps` won't be in
        // scope for this cast):
        //   1. Synthesised `$$ComponentProps`.
        //   2. TS-annotated Props name.
        //   3. User-declared `@typedef {Object} <Name>` block.
        //   4. Svelte-4 `export let` literal shape from PropsInfo.
        //   5. `any` cast.
        let script = doc
            .instance_script
            .as_ref()
            .map(|s| s.content)
            .unwrap_or("");
        let synthesised_name = if should_synthesise_js_props(props_info, script) {
            synthesise_js_props_typedef_body(props_info).map(|_| "$$ComponentProps".to_string())
        } else {
            None
        };
        let props_expr = match synthesised_name
            .or(name_from_ts)
            .or(name_from_jsdoc)
            .or(literal_from_exports)
        {
            Some(body) => format!("/** @type {{{body}}} */({{}})"),
            None => "/** @type {any} */({})".to_string(),
        };
        let _ = writeln!(buf, "    return {{ props: {props_expr} }};");
        return;
    }
    // TS overlay path. Emit a structured return `{ props, events,
    // slots, exports, bindings }` whose field types drive the default
    // export's `Awaited<ReturnType<typeof $$render>>['props']`
    // extraction at module scope — matches upstream's
    // `__sveltets_2_isomorphic_component($$render())` pattern.
    let exports_field = exports_object.unwrap_or("{}");
    // The events field carries the FINAL `$on` event-object map.
    // Matches upstream svelte2tsx's
    // `ComponentEvents.toDefString()` = `'{} as unknown as $$Events'`:
    // `$$Events` is the consumer-facing map, NOT a detail-shape map.
    // Users who want `CustomEvent<…>` wrapping write it explicitly in
    // their `interface $$Events`. The synthesized typed-dispatcher
    // case (`createEventDispatcher<T>()`) is wrapped ONCE at
    // synthesis time (see `synthesized_events_type` in
    // emit/lib.rs's render emission) so the same contract holds
    // regardless of source. `__svn_ensure_component`'s marker
    // branch then uses E directly (no extra wrap), keeping every
    // consumer path consistent at one wrap level.
    let events_field: String = if has_strict_events(doc) || synthesized_events_type.is_some() {
        "$$Events".to_string()
    } else {
        // Lax shape: when no `$$Events` interface is declared, every
        // `on:NAME` handler's payload type defaults to
        // `CustomEvent<any>`. Mirrors upstream's
        // `__sveltets_2_with_any_event` fallback.
        "{ [evt: string]: CustomEvent<any> }".to_string()
    };
    // The `slots:` field literal is written straight into the emit
    // buffer at its splice site — see [`write_slots_field_type`] for
    // shape. Single-line output, so bypassing EmitBuffer's line
    // tracker via `raw_string_mut()` is safe.
    if generics.is_some() {
        let Some(ty) = prop_type_source else {
            return;
        };
        let _ = write!(
            buf,
            "    return {{ props: undefined as any as ({ty}), events: undefined as any as {events_field}, slots: ",
        );
        write_slots_field_type(buf.raw_string_mut(), doc.source, slot_defs);
        let _ = writeln!(
            buf,
            ", bindings: undefined as any as string, exports: undefined as any as ({exports_field}) }};",
        );
        return;
    }
    // No generics. Pick the Props source per priority above. When no
    // Props type was discovered (no `let { x } = $props()`, no
    // SvelteKit-route synthesis, no Svelte-4 `export let` synthesis),
    // fall back to `Record<string, never>` — matches upstream
    // svelte2tsx (`runes-only-export.v5` expectedv2: `props: /** @type
    // {Record<string, never>} */ ({})`). NEVER fall back to the
    // exports object: in Svelte 5 a component can have `export
    // function foo()` (a method exposed via `bind:this`) without
    // exposing any props, and conflating the two surfaces those
    // methods as required props at every consumer site (`Property
    // 'foo' is missing in type '{}' but required in type '{ foo:
    // …; }'`).
    let props_ty: String = prop_type_source
        .map(|ty| ty.to_string())
        .unwrap_or_else(|| "Record<string, never>".to_string());
    let _ = write!(
        buf,
        "    return {{ props: undefined as any as ({props_ty}), events: undefined as any as {events_field}, slots: ",
    );
    write_slots_field_type(buf.raw_string_mut(), doc.source, slot_defs);
    let _ = writeln!(
        buf,
        ", bindings: undefined as any as string, exports: undefined as any as ({exports_field}) }};",
    );
}
