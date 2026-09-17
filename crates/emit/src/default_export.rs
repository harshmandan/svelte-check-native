//! `export default __svn_component_default;` declaration emission.
//!
//! Pulled out of `lib.rs` so the (large) TS path and the (small) JS
//! path can be read together. Two entry points used by the main flow:
//!
//! - [`emit_default_export_declarations_js`] — the JS-overlay shape:
//!   a JSDoc-typed const (`Component<…>` or the shim's
//!   `__SvnIsomorphicComponent<…>`) + `export default`. No interfaces,
//!   no class declarations (TS-only constructs would abort tsgo's
//!   whole-program check on JS overlays).
//! - [`emit_default_export_declarations_ts`] — the TS-overlay shape:
//!   `interface $$IsomorphicComponent`, optional class wrapper for the
//!   generic + Props case, and the Svelte-4 widening intersections.
//!

use std::fmt::Write;

use smol_str::SmolStr;
use svn_parser::Fragment;

use crate::emit_buffer::EmitBuffer;
use crate::svelte4::compat::{fragment_contains_default_slot, fragment_contains_slot};
use crate::util::{generic_arg_names, render_class_name};
use svn_analyze::AmbientRefs;

/// JS-overlay default-export shape — upstream's
/// `addSimpleComponentExport` for a JS file under `emitJsDoc`, written
/// with JSDoc types because a JS overlay cannot hold TS syntax.
///
/// Same choice as the TS path: a runes component without slots or
/// events is a plain Svelte 5 `Component<Props, Exports, Bindings>`
/// (upstream's `__sveltets_2_fn_component`); anything else is the
/// isomorphic component (`__sveltets_2_isomorphic_component[_slots]`),
/// constructible and callable, whose instance carries the component's
/// events and slots.
///
/// For a legacy (non-runes) component the props and slots go through
/// upstream's `__sveltets_2_partial` treatment: an `undefined`-typed
/// entry becomes `any`. A reference to `$$props` / `$$restProps` adds
/// the any-prop index signature (`__sveltets_2_partial_with_any`).
///
/// The const is exported and paired with a same-named `@typedef`, so
/// `import C from './C.svelte'` gives `C` both a value and a type
/// meaning (the instance type), as upstream's declaration merge does.
pub(crate) fn emit_default_export_declarations_js(
    buf: &mut EmitBuffer,
    fragment: &Fragment,
    source: &str,
    render_name: &SmolStr,
    runes_mode: bool,
    has_events: bool,
    ambients: AmbientRefs,
) {
    let render = format!("Awaited<ReturnType<typeof {render_name}>>");
    if runes_mode && !fragment_contains_slot(fragment) && !has_events {
        let _ = writeln!(
            buf,
            "/** @type {{import('svelte').Component<{render}['props'], {render}['exports'], {render}['bindings']>}} */"
        );
        let _ = writeln!(
            buf,
            "export const __svn_component_default = /** @type {{any}} */ (null);"
        );
        let _ = writeln!(
            buf,
            "/** @typedef {{ReturnType<typeof __svn_component_default>}} __svn_component_default */"
        );
        buf.push_str("export default __svn_component_default;\n");
        return;
    }
    let (props, slots) = if runes_mode {
        (format!("{render}['props']"), format!("{render}['slots']"))
    } else {
        (
            format!("__SvnExpand<__SvnPropsAnyFallback<{render}['props']>>"),
            format!("__SvnExpand<__SvnSlotsAnyFallback<{render}['slots']>>"),
        )
    };
    let widened = if ambients.props || ambients.rest_props {
        format!("{props} & __SvnAllProps")
    } else {
        props.clone()
    };
    let props_arg = if fragment_contains_default_slot(fragment, source) {
        format!("__SvnSvelte4SlotedProps<{props}, {widened}>")
    } else {
        widened
    };
    let _ = writeln!(
        buf,
        "/** @type {{__SvnIsomorphicComponent<{props_arg}, {render}['events'], {slots}, {render}['exports'], {render}['bindings']>}} */"
    );
    let _ = writeln!(
        buf,
        "export const __svn_component_default = /** @type {{any}} */ (null);"
    );
    let _ = writeln!(
        buf,
        "/** @typedef {{InstanceType<typeof __svn_component_default>}} __svn_component_default */"
    );
    buf.push_str("export default __svn_component_default;\n");
}

/// TS-overlay default-export shape. Emits an `$$IsomorphicComponent`
/// interface, the matching value/type alias, and (optionally) a class
/// wrapper for generic Props-typed components.
///
/// Mirrors upstream svelte2tsx `addComponentExport.ts:170-179`. Every
/// surface (Props, Events, Slots, Bindings, Exports) flows through
/// either the class wrapper (when `use_class_wrapper` is true) or
/// `Awaited<ReturnType<typeof $$render>>` projections so body-local
/// `typeof X` references resolve inside the render function's scope.
///
/// Class-wrapper: when Props + generics are both present, a
/// `declare class __svn_Render_<hash><T> { props(): Awaited<…>; }` is
/// emitted first. Body-scoped type refs in `$$Props` resolve through
/// the render function's scope without module-scope sanitisation.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_default_export_declarations_ts(
    buf: &mut EmitBuffer,
    fragment: &Fragment,
    source: &str,
    render_name: &SmolStr,
    generics: Option<&str>,
    prop_type_source: Option<&str>,
    has_dispatcher_call: bool,
    has_events: bool,
    has_synth_events_alias: bool,
    has_strict_events_decl: bool,
    runes_mode: bool,
    ambients: AmbientRefs,
) {
    // Upstream's `addComponentExport.ts:343` selects between three
    // default-export shapes. For the **non-generic, runes, no-slots,
    // no-events** profile, upstream emits `__sveltets_2_fn_component`
    // which returns `Component<P, X, B>` — Svelte's actual `Component`
    // interface, callable-only (no `new` ctor). User code that does
    // `Parameters<typeof Comp>` or `(typeof Comp)[]` works cleanly
    // against this shape but breaks against an iso interface (whose
    // `new(...)` ctor sig the inner arrow can't satisfy).
    //
    // Threlte's instancing pattern (gap-A discovery, 2026-04-27) is
    // the canonical example. See `design/gap_a_iso_extraction/` for
    // tsgo-validated repro.
    if should_emit_fn_component_shape(fragment, generics, has_events, runes_mode) {
        // Round-9 follow-up #1: fn-shape doesn't carry the typed-
        // events marker (upstream's `__sveltets_2_fn_component` is a
        // plain `Component<P, X, B>` with no events channel). For
        // type-ref-only typed dispatchers — which round-8 #4 keeps
        // on fn-shape — this means consumer-side `<Comp on:foo>`
        // resolves through `__svn_ensure_component`'s lax untyped
        // overload, matching upstream.
        emit_fn_component_default_export(buf, render_name);
        return;
    }
    let use_class_wrapper = generics.is_some() && prop_type_source.is_some();
    // Class-wrapper declaration at module scope. Its `props()` method's
    // return type is resolved THROUGH the render function, which is
    // where body-local `typeof X` refs are in scope. `Awaited<…>`
    // handles the `async` wrapper on $$render — the body is wrapped
    // in an async function so top-level `await` in user code compiles.
    if use_class_wrapper && let Some(g) = generics {
        let class_name = render_class_name(render_name);
        let g_args = generic_arg_names(g);
        let _ = writeln!(buf, "declare class {class_name}<{g}> {{");
        for field in ["props", "events", "slots", "bindings", "exports"] {
            let _ = writeln!(
                buf,
                "    {field}(): Awaited<ReturnType<typeof {render_name}<{g_args}>>>['{field}'];"
            );
        }
        let _ = writeln!(buf, "}}");
    }

    let prop_ty_root_name = prop_type_source.and_then(svn_analyze::root_type_name_of);
    // v0.3 Item 3: carry the typed event surface as `& { readonly
    // __svn_events: <Events> }` on the default export so
    // `__svn_ensure_component`'s marker branch resolves and
    // narrows `$on(K, cb)` per declared event.
    //
    // Two sources fire this:
    //   (a) Explicit `interface $$Events` / `type $$Events` —
    //       reference `$$Events` at module scope (it's hoisted).
    //   (b) Synthesised `type $$Events = …` from
    //       `createEventDispatcher<T>()` or untyped
    //       `dispatch('name', …)` calls (#3a slice). The synth
    //       lives INSIDE the render body, so we project it back
    //       out via `Awaited<ReturnType<typeof $$render>>['events']`
    //       — same indirection used for props / exports.
    let typed_events_intersection: String = if has_strict_events_decl {
        " & { readonly __svn_events: $$Events }".to_string()
    } else if has_dispatcher_call || has_synth_events_alias {
        // `has_synth_events_alias` covers the bubbled-DOM-only path
        // (reviewer item #3c part 2): a Child with `<button on:click>`
        // and no dispatcher synthesises `$$Events = { "click":
        // HTMLElementEventMap["click"] }` for which the consumer must
        // see the marker so `__svn_ensure_component`'s typed branch
        // fires. `has_dispatcher_call` keeps the marker firing on
        // dispatcher-only / dispatcher+bubbled mixed cases.
        format!(
            " & {{ readonly __svn_events: Awaited<ReturnType<typeof {render_name}>>['events'] }}"
        )
    } else {
        String::new()
    };
    // Conditional index-signature widen mirrors upstream's
    // `__sveltets_2_with_any(…)` factory: adds `SvelteAllProps =
    // {[index: string]: any}` ONLY when the child component refers to
    // `$$props` / `$$restProps` — an identifier reference in a script
    // or a template expression, as upstream's `uses$$props` /
    // `uses$$restProps`. Upstream gates this on
    // `!uses$$Props && (uses$$props || uses$$restProps)` (index.ts:253):
    // a declared `interface/type $$Props` is authoritative, so the
    // AllProps index-signature widen is suppressed. Dropping the
    // `!uses$$Props` term made us accept excess props upstream rejects.
    let uses_any_props =
        (ambients.props || ambients.rest_props) && prop_ty_root_name.as_deref() != Some("$$Props");
    let widen_for = |_base: &str| -> String {
        if uses_any_props {
            " & __SvnAllProps".to_string()
        } else {
            String::new()
        }
    };
    // Upstream's `__sveltets_2_PropsWithChildren<Props, Slots>` adds
    // `children?: any` only when the slots type has a `default` key.
    let has_default_slot = fragment_contains_default_slot(fragment, source);
    // Upstream's `$$IsomorphicComponent` (addComponentExport.ts:170-179):
    // a single interface that types both `new C({props})` (Svelte-4
    // class form) and `C(anchor, props)` (Svelte-5 function form) via a
    // ctor signature + call signature on the same type.
    //
    // CRITICAL (2026-04-25): every TS-overlay component emits this
    // pattern, not just the generic + Props-typed subset. Unifying
    // through the isomorphic pattern + `InstanceType<typeof VALUE>`
    // type alias makes target and value shapes identical by
    // construction (same way upstream does it).
    let class_name = render_class_name(render_name);
    let (props_src, events_src, slots_src, bindings_src, exports_src) =
        if use_class_wrapper && let Some(g) = generics {
            let g_args = generic_arg_names(g);
            (
                format!("ReturnType<{class_name}<{g_args}>['props']>"),
                format!("ReturnType<{class_name}<{g_args}>['events']>"),
                format!("ReturnType<{class_name}<{g_args}>['slots']>"),
                format!("ReturnType<{class_name}<{g_args}>['bindings']>"),
                format!("ReturnType<{class_name}<{g_args}>['exports']>"),
            )
        } else {
            let awaited = format!("Awaited<ReturnType<typeof {render_name}>>");
            (
                format!("{awaited}['props']"),
                format!("{awaited}['events']"),
                format!("{awaited}['slots']"),
                format!("{awaited}['bindings']"),
                format!("{awaited}['exports']"),
            )
        };

    // Upstream widens the render projection itself
    // (`addComponentExport.ts`), never a user-named Props type.
    let widen = widen_for(&props_src);
    let props_typed = format!("{props_src}{widen}");

    // `z_$$bindings` can't reference the interface's own free `<G>`
    // binder — TS interface members aren't under a generic binder.
    // Fill the class/projection's type params with `any` (matches
    // upstream's `toReferencesAnyString()` in Generics.ts).
    let bindings_any_src = if let Some(g) = generics
        && use_class_wrapper
    {
        let g_args = generic_arg_names(g);
        let g_param_count = g_args
            .split(',')
            .filter(|p| !p.trim().is_empty())
            .count()
            .max(1);
        let g_args_any: String = std::iter::repeat_n("any", g_param_count)
            .collect::<Vec<_>>()
            .join(", ");
        format!("ReturnType<{class_name}<{g_args_any}>['bindings']>")
    } else {
        bindings_src.clone()
    };

    let props_wrapped = props_typed.clone();
    // `props_arg` is the Props type seen by consumer-side
    // construction: upstream's `__sveltets_2_PropsWithChildren`
    // shape, mirrored by `__SvnSvelte4SlotedProps<P, Widened>`, when
    // the component has a default slot. It widens to `any` when P is
    // `Record<string, never>` (upstream's own index-signature
    // workaround) and otherwise adds `children?: any`.
    let props_arg: String = if has_default_slot {
        format!("__SvnSvelte4SlotedProps<{props_src}, {props_typed}>")
    } else {
        props_wrapped.clone()
    };
    // The constructor's `SvelteComponent<Props, …>` argument keeps
    // the un-widened wrapped Props — it feeds into TS-level
    // `InstanceType<…>` lookups (component instance shape) where
    // the upstream-style `Partial<…>` form is what the typechecker
    // expects. The widen-to-any short-circuit applies only at the
    // construction-options Props position.
    let svelte_component_props: String = props_wrapped.clone();

    // The CALLABLE return uses `Exports & { $set?: any; $on?: any }`
    // — matches upstream's `__sveltets_2_IsomorphicComponent`'s
    // shape. Without these phantom `$set?`/`$on?` fields, assigning
    // the iso-interface to a bare user-declared `Component<{}, {},
    // string>` (whose callable returns `{ $on?, $set? } & {}`) fails
    // TS2322 because our return doesn't structurally include the
    // required optional fields.
    let g_prefix: String = generics.map(|g| format!("<{g}>")).unwrap_or_default();
    let _ = writeln!(buf, "interface $$IsomorphicComponent {{");
    let _ = writeln!(
        buf,
        "    new {g_prefix}(options: import('svelte').ComponentConstructorOptions<{props_arg}>): import('svelte').SvelteComponent<{svelte_component_props}, {events_src}, {slots_src}> & {{ $$bindings?: {bindings_src} }} & {exports_src};"
    );
    // The call signature takes the props plus the `$$events` /
    // `$$slots` carriers, or only those when the component has no
    // props (upstream's `__sveltets_2_IsomorphicComponent`).
    let carriers = format!("{{ $$events?: {events_src}; $$slots?: {slots_src} }}");
    let _ = writeln!(
        buf,
        "    {g_prefix}(internal: unknown, props: {props_arg} extends Record<string, never> ? {carriers} : {props_arg} & {carriers}): {exports_src} & {{ $set?: any; $on?: any }};"
    );
    let _ = writeln!(buf, "    z_$$bindings?: {bindings_any_src};");
    let _ = writeln!(buf, "}}");
    // `__svn_events` marker keeps the typed-events overload in
    // `__svn_ensure_component` dispatching correctly for children
    // declaring `interface $$Events`.
    let _ = writeln!(
        buf,
        "const __svn_component_default: $$IsomorphicComponent{typed_events_intersection} = null as any;"
    );
    if let Some(g) = generics {
        let g_args = generic_arg_names(g);
        let _ = writeln!(
            buf,
            "type __svn_component_default<{g}> = InstanceType<typeof __svn_component_default<{g_args}>>;"
        );
    } else {
        let _ = writeln!(
            buf,
            "type __svn_component_default = InstanceType<typeof __svn_component_default>;"
        );
    }

    buf.push_str("export default __svn_component_default;\n");
}

/// Should the TS-overlay default export use upstream's
/// `__sveltets_2_fn_component` shape (returns Svelte's `Component<P,
/// X, B>` interface, callable-only) instead of the per-component
/// `$$IsomorphicComponent` interface?
///
/// Mirrors upstream `addComponentExport.ts:343`:
///
/// ```text
/// exportedNames.isRunesMode() && !usesSlots && !events.hasEvents()
/// ```
///
/// `has_events` is upstream's `events.hasEvents()` — whether the
/// component's event map has any entry. A declared `$$Events` is the
/// only source when present and contributes the events it names (an
/// empty interface or a `type $$Events = Base` alias names none);
/// otherwise the entries come from typed dispatchers with inline
/// members, untyped dispatchers called with a literal name, and
/// bubbled events. A dispatcher that is created but never produces a
/// name contributes nothing.
///
/// Runes mode alone decides the rest: `<svelte:options strictEvents>`,
/// `$$props`, `export let` and exported locals do not affect the gate.
///
/// The Component<> shape's lack of a `new(...)` ctor is what makes
/// `Parameters<typeof Comp>` and `(typeof Comp)[]` user patterns work
/// — the inner arrow type satisfies the call signature but cannot
/// satisfy a `new` ctor, so the iso interface fires false-positive
/// TS2322s on those patterns.
fn should_emit_fn_component_shape(
    fragment: &Fragment,
    generics: Option<&str>,
    has_events: bool,
    runes_mode: bool,
) -> bool {
    if generics.is_some() {
        return false;
    }
    if !runes_mode {
        return false;
    }
    if fragment_contains_slot(fragment) {
        return false;
    }
    !has_events
}

/// Emit `Component<P, X, B>` default export — the
/// `__sveltets_2_fn_component`-equivalent shape.
///
/// `Bindings` is passed as `''` (empty literal) to satisfy Svelte's
/// `Bindings extends keyof Props | ''` constraint without requiring
/// per-binding-name tracking. Loses no information today: our render
/// fn types `bindings` as `string` regardless of declared binds, so
/// projecting through wouldn't add detail.
///
/// Round-9 follow-up #1: the fn-shape NEVER carries an `__svn_events`
/// marker. Upstream's `__sveltets_2_fn_component` returns a plain
/// `Component<P, X, B>` with no events channel — consumer-side `$on`
/// resolves through the lax `(event: string, handler) => any`
/// overload. Pre-fix native attached the marker when any synth
/// events surface existed, which was wrong for the type-ref-only
/// typed dispatcher case (kept on fn-shape post round-8 #4 but
/// previously got the marker, narrowing `$on` more strictly than
/// upstream). Bubbled-DOM events disqualify fn-shape entirely
/// (round-8 #4's gate routes them to iso shape), so the original
/// "bubbled-DOM-event narrow-path" rationale for the marker is
/// satisfied at the iso shape's marker emit site, not here.
fn emit_fn_component_default_export(buf: &mut EmitBuffer, render_name: &SmolStr) {
    let _ = writeln!(
        buf,
        "const __svn_component_default: import('svelte').Component<"
    );
    let _ = writeln!(
        buf,
        "    Awaited<ReturnType<typeof {render_name}>>['props'],"
    );
    let _ = writeln!(
        buf,
        "    Awaited<ReturnType<typeof {render_name}>>['exports'],"
    );
    // R-Conv #19 (D-ii fix #4): project the `bindings` field from the
    // render fn instead of a hardcoded `''`. The render fn now emits
    // `__svn_$$bindings('a', 'b')` for runes-mode components (literal
    // union of `$bindable()` prop names) — projecting it through the
    // 3rd generic threads that union to consumer-side `inst.$$bindings
    // = 'NAME'` post-instance checks, where TS2322 fires when NAME
    // isn't bindable.
    let _ = writeln!(
        buf,
        "    Awaited<ReturnType<typeof {render_name}>>['bindings']"
    );
    let _ = writeln!(buf, "> = null as any;");
    let _ = writeln!(
        buf,
        "type __svn_component_default = ReturnType<typeof __svn_component_default>;"
    );
    buf.push_str("export default __svn_component_default;\n");
}
