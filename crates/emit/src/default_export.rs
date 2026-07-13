//! `export default __svn_component_default;` declaration emission.
//!
//! Pulled out of `lib.rs` so the (large) TS path and the (small) JS
//! path can be read together. Two entry points used by the main flow:
//!
//! - [`emit_default_export_declarations_js`] — the JS-overlay shape:
//!   a JSDoc-typed `Component<__SvnDefaultProps>` const + `export
//!   default`. No interfaces, no class declarations (TS-only constructs
//!   would abort tsgo's whole-program check on JS overlays).
//! - [`emit_default_export_declarations_ts`] — the TS-overlay shape:
//!   `interface $$IsomorphicComponent`, optional class wrapper for the
//!   generic + Props case, and the Svelte-4 widening intersections.
//!
//! Plus two byte-scan helpers used to decide whether a Props type
//! reference is module-scope-visible (`module_script_declares_type`,
//! `imports_name`).

use std::fmt::Write;

use smol_str::SmolStr;
use svn_parser::{Document, Fragment};

use crate::emit_buffer::EmitBuffer;
use crate::process_instance_script_content;
use crate::svelte4::compat::{
    contains_export_let, fragment_contains_slot, is_runes_mode, is_svelte4_component,
};
use crate::util::{generic_arg_names, render_class_name};

/// JS-overlay default-export shape. Captures Props via
/// `Awaited<ReturnType<typeof $$render>>['props']` so consumer
/// overlays see real per-element prop types (e.g. the user's local
/// `@typedef {Object} Props` JSDoc) — not the previous loose
/// `Record<string, any>` which let every excess prop silently pass.
/// Closes ~40 TS2353 under-fire sites on a real-world CMS bench.
///
/// `$$render` was modified earlier to `return { props: /** @type
/// {PropsName} */({}) }` when PropsInfo provided a root name;
/// otherwise it returns an empty object literal and the extracted
/// type falls back to `{}` which degrades gracefully (no excess-prop
/// check but no regression).
///
/// TS-only machinery (`interface`, `declare class`, type
/// intersections) is intentionally absent here since those parse
/// errors abort tsgo's whole-program check on JS overlays. See
/// `design/js_overlay/fixture/src/03_default_export.svelte.svn.js`
/// for the vetted shape.
///
/// `| null` in the const's type would cause downstream
/// `__svn_ensure_component(C)` calls to skip the strict
/// `Component<P>` overload and fall through to the
/// `unknown → props?: any` overload — masking excess-prop checks at
/// every consumer. Use a double-cast so the const's TYPE is
/// `Component<Props>` while its runtime VALUE is `null` (no actual
/// runtime needed in a .d.ts-esque overlay).
///
/// The const is `export const` (not bare) and is followed by a
/// matching `@typedef ReturnType<typeof X> X` so the same identifier
/// has both value and type meaning. Without that pair, a consumer
/// doing `import C from "./foo.svelte"` followed by `const x: C = …`
/// fires TS2749 ("C refers to a value, but is being used as a type
/// here"). The named export carries the type alias through the
/// `.d.svelte.ts` sidecar's `export *` re-export, and the dual
/// meaning rides on the default identifier itself — `ReturnType` is
/// the right utility because `Component<P>` is callable-only (no ctor
/// signature), matching the TS fn-component path's choice in
/// [`emit_fn_component_default_export`].
pub(crate) fn emit_default_export_declarations_js(buf: &mut EmitBuffer, render_name: &SmolStr) {
    let _ = writeln!(
        buf,
        "/**\n * @typedef {{Awaited<ReturnType<typeof {render_name}>>['props']}} __SvnDefaultProps\n */"
    );
    // Project the render return's `exports` surface into `Component`'s
    // second type parameter — `ReturnType<Component<P, X>>` is
    // `{ $on?; $set? } & X`, so instance members (`export function` /
    // `export const` / accessors) type precisely at consumers instead
    // of widening to `any` through the default `Exports = {}`.
    // Mirrors upstream, whose JS path feeds createExportsStr()'s
    // exports field into the isomorphic-component projection. Shape
    // validated at design/js_render_full_projection/.
    let _ = writeln!(
        buf,
        "/**\n * @typedef {{Awaited<ReturnType<typeof {render_name}>>['exports']}} __SvnDefaultExports\n */"
    );
    let _ = writeln!(
        buf,
        "/** @type {{import('svelte').Component<__SvnDefaultProps, __SvnDefaultExports>}} */"
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
    doc: &Document<'_>,
    fragment: &Fragment,
    split: Option<&process_instance_script_content::SplitScript>,
    render_name: &SmolStr,
    generics: Option<&str>,
    prop_type_source: Option<&str>,
    template_type_refs: &[SmolStr],
    has_dispatcher_call: bool,
    has_concrete_dispatcher_events: bool,
    has_synth_events_alias: bool,
    has_strict_events_decl: bool,
    has_bubbled_events: bool,
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
    // Type-position reference for every type-only import that was
    // consumed only inside a template expression (`{foo(item as
    // AppVideo)}`). Emitted BEFORE the default-export selection so
    // both the `__sveltets_2_fn_component` path and the
    // `$$IsomorphicComponent` path keep these imports visibly used —
    // without it, the fn_component path early-returns and the import
    // fires TS6133.
    if !template_type_refs.is_empty() {
        buf.push_str("type __svn_tpl_type_refs = [");
        for (i, name) in template_type_refs.iter().enumerate() {
            if i > 0 {
                buf.push_str(", ");
            }
            buf.push_str(name.as_str());
        }
        buf.push_str("];\n");
        buf.push_str("void (0 as any as __svn_tpl_type_refs);\n");
    }
    if should_emit_fn_component_shape(
        doc,
        fragment,
        generics,
        has_concrete_dispatcher_events,
        has_strict_events_decl,
        has_bubbled_events,
    ) {
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

    // The Props source has to be "safe" to reference at module scope:
    // either a literal shape (`{ item: T }`) or a named type whose
    // declaration was hoisted by process_instance_script_content. Bare named types that
    // stay body-scoped (either because they reference the script's
    // generic without re-binding it, or because they reference a
    // body-level const via `typeof`) can't be named from the
    // default-export declaration — emit falls back to `any` for those.
    let prop_ty_is_literal = prop_type_source
        .is_some_and(|t| t.trim().starts_with('{') && !svn_analyze::contains_typeof_ref(t));
    let prop_ty_root_name = prop_type_source.and_then(svn_analyze::root_type_name_of);
    // Consider a named Props type module-scope-visible if either (a)
    // process_instance_script_content hoisted it out of the instance script, (b) it's
    // declared in the `<script module>` section, or (c) it's imported
    // as a type at the module top level.
    let module_script_text = doc.module_script.as_ref().map(|s| s.content).unwrap_or("");
    let prop_ty_module_visible = prop_ty_root_name.as_deref().is_some_and(|n| {
        // `$$ComponentProps` is the reserved name for our TS-source
        // hard-mode synthesis. When the name appears as the Props
        // root, the corresponding `type $$ComponentProps = …;` alias
        // was already emitted at module scope by the same pass — so
        // treat it as module-visible without a hoisted-types lookup.
        n == "$$ComponentProps"
            || split.is_some_and(|s| s.hoisted_type_names.contains(n))
            || module_script_declares_type(module_script_text, n)
            || split.is_some_and(|s| imports_name(&s.hoisted, n))
    });
    let ty_safe_in_generic_scope = prop_ty_is_literal || prop_ty_module_visible;

    // SVELTE-4-COMPAT detection. Consumers of Svelte-4 components pass
    // `on:event` directives (rewritten to `on<event>` prop keys by us)
    // and `<Foo slot="x">` slot-name attrs, neither of which are
    // declared in the actual Props type. Widening with an
    // `on${string}` index signature + optional `slot` key keeps those
    // consumer writes valid without opening the door on Svelte-5
    // codebases where widening would mask real typos.
    let has_slot = fragment_contains_slot(fragment);
    let svelte4_style = is_svelte4_component(doc, split, has_slot);
    let _has_export_let = doc
        .instance_script
        .as_ref()
        .is_some_and(|s| contains_export_let(s.content))
        || doc
            .module_script
            .as_ref()
            .is_some_and(|s| contains_export_let(s.content));
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
    // {[index: string]: any}` ONLY when the child component uses
    // `$$props` / `$$restProps`. Scan the WHOLE document source — a
    // Svelte 4 component can spread `{...$$props}` in the TEMPLATE.
    // Upstream gates this on `!uses$$Props && (uses$$props || uses$$restProps)`
    // (index.ts:253): a declared `interface/type $$Props` is authoritative,
    // so the AllProps index-signature widen is suppressed. Dropping the
    // `!uses$$Props` term made us accept excess props upstream rejects.
    let uses_any_props = (doc.source.contains("$$props") || doc.source.contains("$$restProps"))
        && prop_ty_root_name.as_deref() != Some("$$Props");
    let has_slots = svelte4_style && has_slot;
    let widen_for = |base: &str| -> String {
        if !svelte4_style {
            return String::new();
        }
        match (has_slots, uses_any_props) {
            (false, false) => String::new(),
            (true, false) => format!(" & __SvnSvelte4PropsWiden<{base}>"),
            (false, true) => " & __SvnAllProps".to_string(),
            (true, true) => format!(" & __SvnSvelte4PropsWiden<{base}> & __SvnAllProps"),
        }
    };
    let svelte4_with_slot = svelte4_style && has_slot;
    let wrap_props = |inner: String| -> String {
        if svelte4_with_slot {
            format!("Partial<{inner}>")
        } else {
            inner
        }
    };
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

    // Widen base: if the user named a Props type and it's safe at
    // module scope, use the named type for widening (better error
    // messages at consumer sites — `ChartProps` vs `ReturnType<…>`).
    let widen_base: &str = prop_type_source
        .filter(|_| ty_safe_in_generic_scope)
        .unwrap_or(&props_src);
    let widen = widen_for(widen_base);
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

    let props_wrapped = wrap_props(props_typed.clone());
    // `props_arg` is the Props type seen by consumer-side
    // construction. For Svelte-4 components with a default slot we
    // mirror upstream's `__sveltets_2_PropsWithChildren` shape via
    // `__SvnSvelte4SlotedProps<P, Widened>`: it widens to `any` when
    // P is `Record<string, never>`, sidestepping the index-signature
    // trap that fires `Type '{ children: () => any }' is not
    // assignable to 'Partial<Record<string, never>> & { children?:
    // any }'` on consumers passing implicit-children to an
    // empty-Props Svelte-4 component (e.g. `<StartLayout>...</StartLayout>`
    // where `StartLayout` declares no `export let` / no `$props()`
    // but takes a `<slot/>`). See upstream svelte-shims-v4.d.ts:258-266
    // for the same trap and same workaround.
    //
    // For the non-svelte4-slot path we keep the existing inline
    // shape: `props_wrapped` (already includes Partial<…> when
    // svelte4_with_slot, identity otherwise) plus the conditional
    // `& { children?: any }` for has_slot=true. Svelte-5 components
    // declare `children` in their Props directly, so this branch
    // doesn't need the widen-to-any short-circuit.
    let props_arg: String = if svelte4_with_slot {
        format!("__SvnSvelte4SlotedProps<{props_src}, {props_typed}>")
    } else if has_slot {
        format!("{props_wrapped} & {{ children?: any }}")
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
    let _ = writeln!(
        buf,
        "    {g_prefix}(internal: unknown, props: {props_arg}): {exports_src} & {{ $set?: any; $on?: any }};"
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

    // (template_type_refs emitted above, before the default-export
    // selection — see the top of this function.)
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
/// Where `events.hasEvents()` (`ComponentEvents.ts`) is true if any
/// of: declared `$$Events` interface/type, a typed
/// `createEventDispatcher<T>()` whose `<T>` contributes properties,
/// an untyped dispatcher whose `dispatch('name', …)` calls supply a
/// string-literal first arg, OR a bubbled DOM/component event. We
/// pass the equivalent set in as `has_strict_events_decl ||
/// has_concrete_dispatcher_events || has_bubbled_events`.
///
/// Round-7 follow-up #6: `has_concrete_dispatcher_events` (computed
/// from `synthesized_events_type.is_some()` upstream of the call)
/// only fires when there's a real event source — typed dispatcher
/// with a type arg, or untyped dispatcher with at least one
/// string-literal `dispatch('name', …)` call. Pre-fix native passed
/// `has_dispatcher_call` here, which fires for ANY
/// `createEventDispatcher()` call site regardless of whether it
/// produces actual events. A runes component that creates a
/// dispatcher but never dispatches anything (or doesn't supply a
/// type arg) was wrongly disqualified from the fn-component shape.
///
/// Round-6 follow-up #3: pre-fix the native gate also blocked on
/// `<svelte:options strictEvents />` (no-op in runes mode),
/// `$$slots`/`$$restProps`/`$$props` substrings (Svelte-4 features
/// that runes mode disallows anyway), `export let` in the instance
/// or module script, and any non-empty `exported_locals`. None of
/// those affect upstream's gate, so they were drift — they pushed
/// otherwise-eligible runes components onto the iso shape and broke
/// `Parameters<typeof Comp>` / `(typeof Comp)[]` patterns that
/// require the callable-only Component<> form.
///
/// The Component<> shape's lack of a `new(...)` ctor is what makes
/// `Parameters<typeof Comp>` and `(typeof Comp)[]` user patterns work
/// — the inner arrow type satisfies the call signature but cannot
/// satisfy a `new` ctor, so the iso interface fires false-positive
/// TS2322s on those patterns.
fn should_emit_fn_component_shape(
    doc: &Document<'_>,
    fragment: &Fragment,
    generics: Option<&str>,
    has_concrete_dispatcher_events: bool,
    has_strict_events_decl: bool,
    has_bubbled_events: bool,
) -> bool {
    if generics.is_some() {
        return false;
    }
    if !is_runes_mode(doc, fragment) {
        return false;
    }
    if fragment_contains_slot(fragment) {
        return false;
    }
    // events.hasEvents() — declared interface, typed/untyped
    // dispatcher contributing concrete event names, or bubbled
    // DOM/component events. Upstream's gate folds all three sources
    // behind one boolean; we maintain three booleans so we can apply
    // each at the right emit site, but at the gate they collapse to
    // the same OR. The dispatcher signal is `concrete_dispatcher_events`
    // (Some(synthesized_events_type)), not the broader "any
    // createEventDispatcher() exists" — a dispatcher with no type arg
    // and no actual `dispatch('name', …)` calls produces zero events.
    if has_strict_events_decl || has_concrete_dispatcher_events || has_bubbled_events {
        return false;
    }
    true
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

/// Byte-scan a script section for `type NAME`, `interface NAME`,
/// `export type NAME`, or `export interface NAME` declarations.
/// Returns true if `name` appears as a declared type.
///
/// Not a full parser — matches the common case of a top-of-line type
/// keyword followed by whitespace and the identifier. String-literal
/// and comment false-positives resolve toward "visible"; emit then
/// declares `Component<Foo>` which fires TS2304 only if the user
/// genuinely forgot to declare Foo, a clear error they can fix.
fn module_script_declares_type(text: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    for prefix in ["type ", "interface "] {
        let needle = format!("{prefix}{name}");
        for (idx, _) in text.match_indices(&needle) {
            let before_ok = idx == 0 || {
                let b = text.as_bytes()[idx - 1];
                !b.is_ascii_alphanumeric() && b != b'_' && b != b'$'
            };
            let after_idx = idx + needle.len();
            let after_ok = after_idx == text.len() || {
                let b = text.as_bytes()[after_idx];
                !b.is_ascii_alphanumeric() && b != b'_' && b != b'$'
            };
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
}

/// Byte-scan hoisted import declarations for `name` appearing in an
/// `import type { ... }` or `import { ... }` clause. Used to check
/// whether a Props type referenced by the consumer emit comes from a
/// type-only module import.
fn imports_name(hoisted: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // Fast path: check any `import` statement that mentions the name
    // as a braced specifier. Matches `{ name }`, `{ name, ... }`,
    // `{ ..., name }`, `{ name as Alias }`, `{ type name }`, etc.
    for (idx, _) in hoisted.match_indices(name) {
        let before = idx.checked_sub(1).map(|i| hoisted.as_bytes()[i]);
        let after = hoisted.as_bytes().get(idx + name.len()).copied();
        let bounded = before.is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'_' && b != b'$')
            && after.is_none_or(|b| !b.is_ascii_alphanumeric() && b != b'_' && b != b'$');
        if !bounded {
            continue;
        }
        // Scan backward to the nearest `import`, stopping at a prior
        // `;` or `\n\n` (statement boundary). If we hit `import` with
        // an open `{` between it and the name, it's an import
        // specifier.
        let before_text = &hoisted[..idx];
        if let Some(import_pos) = before_text.rfind("import") {
            let between = &before_text[import_pos + "import".len()..];
            if between.contains('{') && !between.contains('}') {
                return true;
            }
        }
    }
    false
}
