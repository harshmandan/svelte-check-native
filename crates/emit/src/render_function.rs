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
use crate::process_instance_script_content::ExportedLocalInfo;
use crate::props_emit::{synthesise_js_props_typedef_body, write_slots_field_type};
use crate::svelte4;
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
    has_strict_slots_decl: bool,
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
    // R-Conv #20 (B2 #3): when the template has any `<slot>` element,
    // declare `__svn_create_slot` once at the top of the check body
    // so per-slot emit downstream can call it. With `interface
    // $$Slots` declared, the helper's generic narrows to it; without,
    // the `Record<string, Record<string, any>>` default keeps Svelte-4
    // components silent. Mirrors upstream svelte2tsx's `;const
    // __sveltets_createSlot = __sveltets_2_createCreateSlot<$$Slots>();`
    // emission at `htmlxtojsx_v2/nodes/Slot.ts` + `addComponentExport.ts`.
    if is_ts && svelte4::compat::fragment_contains_slot(fragment) {
        if has_strict_slots_decl {
            buf.push_str(
                "        const __svn_create_slot = __svn_create_create_slot<$$Slots>();\n",
            );
        } else {
            buf.push_str("        const __svn_create_slot = __svn_create_create_slot();\n");
        }
    }
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

/// Emit `$$render_<hash>`'s return statement at the tail of its body.
/// Called unconditionally for every overlay shape; the body picks the
/// branch:
///   - JS overlay → `return { props: /** @type … */({}) }`.
///   - generics declared → structured `{ props, events, slots,
///     bindings, exports }` whose surfaces a sibling
///     `declare class __svn_Render_<hash>` projects back out at module
///     scope (the `use_class_wrapper` case).
///   - Svelte-4 `interface $$Props` → props spread through
///     `__svn_ensure_right_props<…>`.
///   - plain (no generics) → structured return with the discovered
///     Props type or `Record<string, never>` fallback.
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
    synth_events_alias_body: Option<&str>,
    exports_object: Option<&str>,
    export_type_infos: &[ExportedLocalInfo],
    dollar_props_name_range: Option<svn_core::Range>,
    props_info: &svn_analyze::PropsInfo,
    slot_defs: &[svn_analyze::SlotDef],
    has_strict_events_decl: bool,
    has_strict_slots_decl: bool,
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
    // synthesis time, then intersected with any bare-DOM-event
    // bubble projection (`<button on:click>` → `{ "click":
    // HTMLElementEventMap["click"] }`) — see emit/lib.rs's
    // render emission. `synth_events_alias_body` is non-None when
    // EITHER half fired, so the gate covers the full synthesised
    // surface. `__svn_ensure_component`'s marker branch then uses E
    // directly (no extra wrap), keeping every consumer path
    // consistent at one wrap level.
    let events_field: String = if has_strict_events_decl || synth_events_alias_body.is_some() {
        "$$Events".to_string()
    } else {
        // Lax shape: when no `$$Events` interface is declared, every
        // `on:NAME` handler's payload type defaults to
        // `CustomEvent<any>`. Mirrors upstream's
        // `__sveltets_2_with_any_event` fallback.
        "{ [evt: string]: CustomEvent<any> }".to_string()
    };
    // The `bindings:` field type carries the literal-string union of
    // bindable prop names per upstream svelte2tsx's
    // `createBindingsStr` (ExportedNames.ts:764-771):
    //   - Runes mode: `__svn_$$bindings('a', 'b')` — typed as
    //     `'a' | 'b'`. Drives TS2322 on
    //     `inst.$$bindings = '<not-bindable>'` post-instance checks.
    //   - Svelte-4 mode: `string` — every `export let` /
    //     `export function` is bindable, so the iso ctor's
    //     `$$bindings?: string` accepts any name.
    let bindings_field: String = build_bindings_field(props_info);
    // The `slots:` field literal is written straight into the emit
    // buffer at its splice site — see [`write_slots_field_type`] for
    // shape. Single-line output, so bypassing EmitBuffer's line
    // tracker via `raw_string_mut()` is safe.
    // SlotHandler PLAN Stage 5: when the user declared `interface
    // $$Slots` / `type $$Slots` in the instance script, their
    // declaration is authoritative — emit `undefined as any as
    // $$Slots` instead of the synthesised slot-defs (mirrors
    // upstream's `uses$$SlotsInterface` behavior at
    // `createRenderFunction.ts:125-133`).
    let write_slots_field = |out: &mut String| {
        if has_strict_slots_decl {
            out.push_str("undefined as any as $$Slots");
        } else {
            write_slots_field_type(out, doc.source, slot_defs);
        }
    };
    if generics.is_some() {
        // When generics are declared but no Props source was discovered,
        // fall back to `Record<string, never>` just like the no-generics
        // path below — an early return here would leave the render fn body
        // returnless, so the default-export projection
        // (`Awaited<ReturnType<typeof $$render>>['props']`) would resolve
        // to `void` and break every consumer.
        let props_ty: String = match prop_type_source {
            Some(ty) => ty.to_string(),
            None => "Record<string, never>".to_string(),
        };
        let _ = write!(
            buf,
            "    return {{ props: undefined as any as ({props_ty}), events: undefined as any as {events_field}, slots: ",
        );
        write_slots_field(buf.raw_string_mut());
        let _ = writeln!(
            buf,
            ", bindings: {bindings_field}, exports: undefined as any as ({exports_field}) }};",
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
    //
    // Svelte-4 `interface $$Props` cross-check — mirrors upstream
    // `ExportedNames.createPropsStr`'s `uses$$Props` branch. Spreads
    // an empty-typed call into `__svn_ensure_right_props<{<lets>}>(
    // __svn_any("") as $$Props)` so TS fires TS2345 when `$$Props`
    // is wider/narrower than the declared `export let X: T` shape.
    if matches!(props_info.source, svn_analyze::PropsSource::LegacyInterface) {
        let lets_shape: String = build_exported_lets_shape(export_type_infos);
        let _ = write!(
            buf,
            "    return {{ props: {{ ...__svn_ensure_right_props<{lets_shape}>("
        );
        // Anchor the cast expression to the source's `$$Props`
        // declaration name. TS2345 fired on the type-assertion
        // argument reverse-maps onto the interface's name span,
        // matching upstream LS's `movePropsErrorRangeBackIfNecessary`.
        let cast = "__svn_any(\"\") as $$Props";
        if let Some(range) = dollar_props_name_range {
            buf.append_with_source(cast, range);
        } else {
            buf.push_str(cast);
        }
        let _ = write!(
            buf,
            ") }} as $$Props, events: undefined as any as {events_field}, slots: ",
        );
        write_slots_field(buf.raw_string_mut());
        let _ = writeln!(
            buf,
            ", bindings: {bindings_field}, exports: undefined as any as ({exports_field}) }};",
        );
        return;
    }
    let props_ty: String = prop_type_source
        .map(|ty| ty.to_string())
        .unwrap_or_else(|| "Record<string, never>".to_string());
    let _ = write!(
        buf,
        "    return {{ props: undefined as any as ({props_ty}), events: undefined as any as {events_field}, slots: ",
    );
    write_slots_field(buf.raw_string_mut());
    let _ = writeln!(
        buf,
        ", bindings: {bindings_field}, exports: undefined as any as ({exports_field}) }};",
    );
}

/// Build the `bindings:` field expression for the render-fn return.
/// Mirrors upstream svelte2tsx's `createBindingsStr`
/// (`ExportedNames.ts:764-771`):
///
/// - Runes mode (`PropsSource::RuneAnnotation` /
///   `PropsSource::RuneGeneric`): collect every destructure entry with
///   `is_bindable: true` (the `$bindable()` marker on the default) and
///   emit `__svn_$$bindings('a', 'b')`. The shim returns
///   `Bindings[number]`, i.e. the literal-string union `'a' | 'b'`.
///   Drives TS2322 on `inst.$$bindings = '<not-bindable>'` post-
///   instance checks.
/// - Svelte-4 mode (any other source): emit `undefined as any as
///   string`. Every `export let` / `export function` is bindable, so
///   the iso ctor's `$$bindings?: string` accepts any name and the
///   post-instance check stays silent.
fn build_bindings_field(props_info: &svn_analyze::PropsInfo) -> String {
    let is_runes = matches!(
        props_info.source,
        svn_analyze::PropsSource::RuneAnnotation
            | svn_analyze::PropsSource::RuneGeneric
            | svn_analyze::PropsSource::SynthesisedFromDestructure
    );
    if !is_runes {
        return "undefined as any as string".to_string();
    }
    // `local_only` leaves (nested-pattern `$bindable`s) never reach
    // upstream's bindings list — its loop only reads simple elements.
    let bindable: Vec<&svn_analyze::PropInfo> = props_info
        .destructures
        .iter()
        .filter(|p| p.is_bindable && !p.local_only)
        .collect();
    if bindable.is_empty() {
        // Runes-mode component with no `$bindable()` props — emit
        // `__svn_$$bindings()` returning `never`. Any `bind:NAME`
        // post-instance check fires TS2322 against `never`. Mirrors
        // upstream's `__sveltets_$$bindings('')` empty-string call
        // (which returns `''`, the empty literal type), but our
        // helper signature uses `Bindings[number]` so a no-arg call
        // returns `never` — strictly equivalent in firing TS2322 on
        // any `inst.$$bindings = '<NAME>'` assignment.
        return "__svn_$$bindings()".to_string();
    }
    let mut out = String::from("__svn_$$bindings(");
    let mut first = true;
    for p in bindable {
        if !first {
            out.push_str(", ");
        }
        first = false;
        out.push('\'');
        // PropInfo.prop_key is the public name (matches `bind:NAME`).
        out.push_str(p.prop_key.as_str());
        out.push('\'');
    }
    out.push(')');
    out
}

/// Build the `{ X: T, Y?: U, ... }` type literal for every `export let`
/// declaration. Mirrors upstream svelte2tsx's
/// `createReturnElementsType(lets)` (`ExportedNames.ts:759-784`):
///
/// - Only `isLet` entries participate; `export const` / `export
///   function` go through the separate `exports` field.
/// - Has-init → optional (`?:`); no-init → required (`:`).
/// - Type source: declared annotation > `typeof <name>` when there is
///   no annotation. `typeof <name>` resolves inside the render-fn scope
///   where the stripped-`export` `let X = …` lives, picking up the
///   literal inferred type.
///
/// Returns `{}` when no `export let`s exist — the
/// `__svn_ensure_right_props<{}>` form upstream emits for
/// `interface $$Props` components with no `export let`s
/// (see `ts-$$Props-interface-only-props/expectedv2.ts:14`).
fn build_exported_lets_shape(infos: &[ExportedLocalInfo]) -> String {
    let lets: Vec<&ExportedLocalInfo> = infos.iter().filter(|i| i.is_let).collect();
    if lets.is_empty() {
        return "{}".to_string();
    }
    let mut out = String::from("{");
    let mut first = true;
    for info in lets {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(info.name.as_str());
        if info.has_init {
            out.push('?');
        }
        out.push_str(": ");
        match &info.type_source {
            Some(t) => out.push_str(t),
            None => {
                out.push_str("typeof ");
                out.push_str(info.name.as_str());
            }
        }
    }
    out.push('}');
    out
}
