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
use crate::props_emit::write_slots_field_type;
use crate::svelte4;
use svn_analyze::TemplateSummary;

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
///
/// `root_snippets_hoisted`: the fragment's own `{#snippet}` blocks were
/// already emitted at the render function's start (or at module
/// scope), so the walk skips them here.
pub(crate) fn emit_template_check_fn(
    buf: &mut EmitBuffer,
    doc: &svn_parser::Document<'_>,
    fragment: &svn_parser::Fragment,
    summary: &TemplateSummary,
    is_ts: bool,
    has_strict_slots_decl: bool,
    root_snippets_hoisted: bool,
) {
    let full_fragment = fragment;
    // When the template has any `<slot>` element (root snippets
    // included), declare `__svn_create_slot` once in the render
    // function, outside the check body, where upstream declares
    // `__sveltets_createSlot`: a root snippet that stays in the render
    // function sees it, one hoisted to module scope does not. With
    // `interface $$Slots` declared, the helper's generic narrows to it;
    // without, the `Record<string, Record<string, any>>` default keeps
    // Svelte-4 components silent.
    if svelte4::compat::fragment_contains_slot(full_fragment) {
        if is_ts && has_strict_slots_decl {
            buf.push_str("    const __svn_create_slot = __svn_create_create_slot<$$Slots>();\n");
        } else {
            buf.push_str("    const __svn_create_slot = __svn_create_create_slot();\n");
        }
    }
    let without_root_snippets;
    let fragment = if root_snippets_hoisted {
        without_root_snippets = svn_parser::Fragment {
            nodes: fragment
                .nodes
                .iter()
                .filter(|n| !matches!(n, svn_parser::Node::SnippetBlock(_)))
                .cloned()
                .collect(),
            ..fragment.clone()
        };
        &without_root_snippets
    } else {
        fragment
    };
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
    let instantiations_by_start = instantiation_index(summary);
    let mut action_counter: usize = 0;
    buf.resync_current_line();
    let verbatim = VERBATIM_TEMPLATE_TEXT.with(|v| v.borrow().clone());
    if verbatim.is_empty() {
        emit_template_body(
            buf,
            doc.source,
            fragment,
            2,
            &instantiations_by_start,
            &mut action_counter,
        );
    } else {
        // Script text svelte2tsx does not treat as a script sits among
        // the template's top-level nodes in source order.
        let mut rest: &[svn_parser::Node] = &fragment.nodes;
        for range in &verbatim {
            let split = rest
                .iter()
                .position(|n| n.range().start >= range.end)
                .unwrap_or(rest.len());
            let (before, after) = rest.split_at(split);
            let part = svn_parser::Fragment {
                nodes: before.to_vec(),
                ..fragment.clone()
            };
            emit_template_body(
                buf,
                doc.source,
                &part,
                2,
                &instantiations_by_start,
                &mut action_counter,
            );
            buf.push_str("        ");
            buf.append_with_source(range.slice(doc.source), *range);
            buf.push_str("\n");
            rest = after;
        }
        let part = svn_parser::Fragment {
            nodes: rest.to_vec(),
            ..fragment.clone()
        };
        emit_template_body(
            buf,
            doc.source,
            &part,
            2,
            &instantiations_by_start,
            &mut action_counter,
        );
    }
    buf.push_str("    });\n");
}

thread_local! {
    /// Source ranges of script blocks svelte2tsx leaves in the template
    /// as text (see `verbatim_scripts`), set for one document's emit.
    static VERBATIM_TEMPLATE_TEXT: std::cell::RefCell<Vec<svn_core::Range>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Run `f` with `ranges` written verbatim into the template check body.
pub(crate) fn with_verbatim_template_text<R>(
    ranges: Vec<svn_core::Range>,
    f: impl FnOnce() -> R,
) -> R {
    let previous = VERBATIM_TEMPLATE_TEXT.with(|v| std::mem::replace(&mut *v.borrow_mut(), ranges));
    let out = f();
    VERBATIM_TEMPLATE_TEXT.with(|v| *v.borrow_mut() = previous);
    out
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
    props_info: &svn_analyze::PropsInfo,
    js_props_typedef_synthesised: bool,
    slot_defs: &[svn_analyze::SlotDef],
    has_strict_events_decl: bool,
    has_strict_slots_decl: bool,
    runes_mode: bool,
    ambients: svn_analyze::AmbientRefs,
) {
    // JS overlay: always emit a return so the default-export's
    // `Awaited<ReturnType<typeof $$render>>['props']` extraction
    // resolves to a real Props type. The props expression follows
    // upstream's `ExportedNames.createPropsStr` for a JS component:
    //
    //   - runes mode: the synthesised `$$ComponentProps` typedef, else
    //     the `@type` comment leading the `$props()` declaration, else
    //     `Record<string, never>`;
    //   - legacy mode: the component's exports, else `{}` when it reads
    //     `$$props` / `$$restProps`, else `Record<string, never>`.
    //
    // Neither mode looks at unrelated JSDoc such as a `@typedef` block.
    if !emit_is_ts() {
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
        // Svelte-4 `export let` synthesis: PropsInfo captures a
        // literal `{k: T, …}` type_text (PropsSource::SynthesisedFromExports).
        // Embed the literal body directly as the JSDoc `@type` so the
        // default export's `Awaited<ReturnType<…>>['props']` resolves
        // to the typed shape.
        let literal_from_exports = prop_type_source
            .filter(|_| {
                matches!(
                    props_info.source,
                    svn_analyze::PropsSource::SynthesisedFromExports
                )
            })
            .map(|ty| ty.trim().to_string());
        let never = "/** @type {Record<string, never>} */ ({})".to_string();
        let as_type = |body: String| format!("/** @type {{{body}}} */({{}})");
        let props_expr = if runes_mode {
            if js_props_typedef_synthesised {
                as_type("$$ComponentProps".to_string())
            } else if let Some(name) = name_from_ts {
                as_type(name)
            } else if let Some(comment) = props_info.props_type_comment.as_deref() {
                format!("{comment}({{}})")
            } else {
                never
            }
        } else if let Some(body) = literal_from_exports.or(name_from_ts) {
            as_type(body)
        } else if ambients.props || ambients.rest_props {
            "{}".to_string()
        } else {
            never
        };
        // Full projection, JS-safe: upstream createRenderFunction.ts
        // returns { props, exports, bindings, slots, events } for JS
        // and TS alike. Without the exports field the default
        // export's Exports projection resolves to `{}` and every
        // instance member on a JS component widens to `any` at
        // consumers, so misuse (e.g. comparing a boolean export
        // against a string) goes undiagnosed. JSDoc casts replace
        // the TS-only `undefined as any as T` shape — validated at
        // design/js_render_full_projection/. `$$Slots` interfaces are
        // TS-only declarations, so the slots field is always the
        // synthesised literal. The events field carries the same
        // collected event map a TS component gets (upstream's
        // ComponentEvents does not care about the script language),
        // stated as a JSDoc type; with nothing collected it is the
        // lax index signature.
        let exports_expr = match exports_object {
            Some(o) => format!("/** @type {{{o}}} */ ({{}})"),
            None => "{}".to_string(),
        };
        let events_ty = synth_events_alias_body.unwrap_or("{ [evt: string]: CustomEvent<any> }");
        let bindings_field = build_bindings_field(props_info, false);
        let _ = write!(
            buf,
            "    return {{ props: {props_expr}, events: /** @type {{{events_ty}}} */ ({{}}), slots: ",
        );
        write_slots_field_type(buf.raw_string_mut(), doc.source, slot_defs, false);
        let _ = writeln!(
            buf,
            ", bindings: {bindings_field}, exports: {exports_expr} }};"
        );
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
    let bindings_field: String = build_bindings_field(props_info, true);
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
            write_slots_field_type(out, doc.source, slot_defs, true);
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
    //
    // The call is generated code with no source position, so svelte-check
    // drops that TS2345: the language server moves it onto the `$$Props`
    // declaration, but the `--tsgo` path has no language service to do
    // so and discards the unmapped diagnostic.
    if matches!(props_info.source, svn_analyze::PropsSource::LegacyInterface) {
        let lets_shape: String = build_exported_lets_shape(export_type_infos);
        let _ = write!(
            buf,
            "    return {{ props: {{ ...__svn_ensure_right_props<{lets_shape}>("
        );
        buf.push_str("__svn_any(\"\") as $$Props");
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
///
/// `is_ts` picks the lax-string spelling: JS overlays can't carry the
/// `as` cast, so they emit the JSDoc equivalent `/** @type {string} */
/// ('')` — same `string` field type. The runes-mode
/// `__svn_$$bindings(...)` call is plain JS and shared by both modes.
fn build_bindings_field(props_info: &svn_analyze::PropsInfo, is_ts: bool) -> String {
    let is_runes = matches!(
        props_info.source,
        svn_analyze::PropsSource::RuneAnnotation
            | svn_analyze::PropsSource::RuneGeneric
            | svn_analyze::PropsSource::SynthesisedFromDestructure
    );
    if !is_runes {
        return if is_ts {
            "undefined as any as string".to_string()
        } else {
            "/** @type {string} */ ('')".to_string()
        };
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

/// Component instantiations indexed by the source byte offset of their
/// node, the lookup every component-call emit site uses.
pub(crate) fn instantiation_index(
    summary: &TemplateSummary,
) -> std::collections::HashMap<u32, &svn_analyze::ComponentInstantiation> {
    summary
        .component_instantiations
        .iter()
        .map(|inst| (inst.node_start, inst))
        .collect()
}
