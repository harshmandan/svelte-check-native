/// <reference lib="dom" />
/// <reference lib="dom.iterable" />

// Core Svelte type shims — always shipped into the project cache.
//
// The two reference directives above forcibly include the DOM lib
// regardless of the user's `compilerOptions.lib` setting. Rationale:
// Svelte components always run in a browser-like context (real DOM
// or a minimal polyfill), and the emit references DOM types via
// bind:this handlers, event handlers, `svelteHTML.createElement`
// paths, etc. Projects that narrow `lib` to exotic values like
// `["WebWorker"]` (seen in service-worker-only apps) would otherwise
// lose access to HTMLElement/document/alert/etc. at type-check time,
// firing TS2304 "Cannot find name" on every element binding. Upstream
// svelte-check takes the same approach in svelte-jsx-v4.d.ts; mirror it.
//
//
// Holds the Svelte 5 rune ambients ($state, $derived, $effect, $props,
// $bindable, $inspect, $host) plus the helper types emit references
// (__SvnStoreValue, __svn_type_ref). These have no equivalent in the
// real `svelte` npm package — runes are compiler macros, and the
// helpers are our private contract with the emit crate — so this file
// is written to the cache on every check, regardless of whether the
// user has `svelte` installed in node_modules.
//
// The `@@FALLBACK_BEGIN@@` … `@@FALLBACK_END@@` block below holds
// the `declare module 'svelte/*'` stand-ins for the real package.
// The runtime (typecheck/src/lib.rs) strips the whole block before
// writing the shim into the cache WHEN a real svelte install is
// reachable from the workspace. Without that strip, the fallback
// declarations would shadow the richer real types (e.g.
// `HTMLAnchorAttributes` from svelte/elements) and produce
// false-positive TS2305 errors on user code that uses names the
// fallback doesn't enumerate.

// Runes are declared at top level (script mode) rather than inside
// `declare global` because this file is a `.d.ts` script (no top-level
// imports/exports), so its declarations are already global.

// ---------- helpers used by emit ----------

/** Minimal shape of a Svelte store. */
type __SvnStore<T> = { subscribe: (run: (value: T) => any, invalidate?: any) => any };

/**
 * Type-level store unwrap. Used in emit as
 *   `let $foo!: __SvnStoreValue<typeof foo>;`
 *
 * Forward references the store's *type* without depending on
 * declaration order — the `let` declaration goes ABOVE the body so the
 * body can reference `$foo`, but `foo` itself is declared further down.
 * TS resolves types lazily, so `typeof foo` works even when `foo`
 * appears later in the source.
 *
 * The conditional handles non-store inputs by falling through to the
 * input type itself (matches what Svelte's auto-subscribe would do).
 * `undefined | null` collapse to themselves, which is the closest we can
 * get to the runtime "subscribe-first" semantic without actually
 * calling subscribe.
 */
type __SvnStoreValue<S> =
    S extends __SvnStore<infer T> ? T : S;

/**
 * Surface a type-only template reference inside the type-check function
 * so TS6196 doesn't fire on `import type { Foo }` that's only used in a
 * `<Component prop={value as Foo} />`-style assertion. The body is a
 * pure type expression — no runtime cost.
 */
declare function __svn_type_ref<T>(): T;

// SVELTE-4-COMPAT: the `ConstructorOfATypedSvelteComponent` type is a
// Svelte-4 typing convention from upstream svelte-check's shims. User
// code in mid-migration codebases types props that hold component
// constructors as `export let icon: ConstructorOfATypedSvelteComponent;`
// — the prop receives the class-form component, which consumers
// render via `<svelte:component this={icon} />`. Svelte 5 replaces the
// pattern with `Component<Props>` typing, but until the migration is
// complete we mirror upstream's declaration so tsgo resolves the name
// at the use site rather than firing TS2304.
//
// The shape mirrors upstream verbatim — `$$prop_def`, `$$events_def`,
// `$$slot_def` are compile-time-only fields that never exist at
// runtime; they carry per-component Props / Events / Slots types for
// the Svelte-4 class-form component world. Projects can inspect the
// property types via `ComponentProps<typeof X>` / `ComponentEvents<…>`
// etc. without pulling DOM / browser bindings.
/**
 * @internal This is for type checking capabilities only and does not
 * exist at runtime. Don't use this property.
 */
declare type ATypedSvelteComponent = {
    /** @internal */
    $$prop_def: any;
    /** @internal */
    $$events_def: any;
    /** @internal */
    $$slot_def: any;
    $set(props?: any): void;
    $on(event: string, handler: ((e: any) => any) | null | undefined): () => void;
    $destroy(): void;
    $capture_state(): void;
    $inject_state(): void;
};

/**
 * Constructor type for Svelte-4-style class-form components. Users
 * type props as `ConstructorOfATypedSvelteComponent` when the prop
 * carries a `<Component />` to dynamically render.
 *
 * The strict Svelte-4 shape (`new (args: { target, props? }) =>
 * ATypedSvelteComponent`) doesn't accept Svelte-5-compiled component
 * imports (tabler-icons, lucide-svelte, phosphor-svelte, etc.)
 * because those declare `Component<Props>` function types. Keeping
 * the strict shape fires dozens of false-positive TS2322 assignment
 * errors on any Svelte-4 codebase that imports a Svelte-5 icon
 * library.
 *
 * Widening to a broad "any component constructor" shape matches
 * upstream svelte-check's effective behavior (where the type only
 * surfaces inside `__sveltets_2_ensureComponent`'s union, which
 * accepts both forms). Users still get the name resolved at the use
 * site; the more specific type check was a false-positive
 * generator, not a real safety net — real type errors come from the
 * component's own props declaration, not from this top-level holder.
 */
declare type ConstructorOfATypedSvelteComponent = any;

// SVELTE-4-COMPAT: additive props-type widening for Svelte-4 components.
// A parent's `<Foo on:close={fn}>` is rewritten by our analyze pass to
// `{onclose: fn}` on the child's props object; a parent's
// `<Foo slot="x">` lands as `{slot: "x"}`. Neither key exists on Foo's
// declared Props when Foo uses Svelte-4 `createEventDispatcher` or
// upstream slot syntax. Intersecting `__SvnSvelte4PropsWiden<Props>`
// into the Props type argument of a Svelte-4 component's default
// export silences TS2353 on those keys without opening every
// component to any-prop abuse (only files that trip
// `is_svelte4_component` get the widen).
//
// `Omit<…, keyof P>` is load-bearing: WITHOUT it, a declared prop
// `onChange: (v: string) => void` intersects with the widen's
// `on${string}` signature `(e: CustomEvent<any>) => any`, collapsing
// the union to `never` and rejecting every caller's handler. With
// the Omit, already-declared on* keys pass through unchanged; the
// widen only introduces BRAND-NEW keys (handlers for events the
// component dispatches but doesn't declare as props, like Svelte-4
// `createEventDispatcher` usage).
//
// Handler type is deliberately lax: `CustomEvent<any>` rather than
// `CustomEvent<Detail>` for each specific event name — synthesising
// the exact detail shape from `createEventDispatcher<…>()`
// introspection is a later refinement.
// The handler signature `(e: any) => any` is load-bearing. Narrower
// signatures like `(e: CustomEvent<any>) => any` create an
// index-signature conflict with declared props like
// `onChange: (v: string) => void` — TS reports "Property onChange
// is incompatible with index signature" even when combined via
// `Omit`. Wider values (`any`) avoid the conflict but cause TS7031
// "binding element implicitly has an 'any' type" on destructures
// like `({detail}) => …` because destructuring a raw `any`
// parameter fires implicit-any in strict mode.
//
// `(e: any) => any` threads the needle: the parameter is
// explicitly-`any`-typed, so `({detail})` destructuring
// contextually types `detail: any` (not implicit). And the
// function-to-function assignability check treats `(v: string) =>
// void` as compatible with `(e: any) => any` via bivariance, so the
// index-signature conflict doesn't fire.
// Matches upstream's `__sveltets_2_PropsWithChildren<Props, Slots>`
// shape (svelte-shims-v4.d.ts:258-266) — only adds `children?: any`
// when the component has a default slot. Everything else (class, style,
// slot, on*) must be declared in user Props or users hit TS2353 —
// same strictness as upstream.
//
// Prior version intersected {slot?, class?, style?, children?} +
// {[index: string]: any} unconditionally; ANY non-empty intersection
// contaminated tsgo's assignability check for missing-required-prop
// cases — tsgo reported TS2322 "Type '{}' is not assignable" at the
// top level with the precise TS2741 as a sub-message (observed on
// language-tools/.../test-error/Index.svelte's `<Jsdoc />`). Matching
// upstream's minimal widen lets TS2741 surface directly.
declare type __SvnSvelte4PropsWiden<P> = 'children' extends keyof P
    ? {}
    : { children?: any };

// Applied CONDITIONALLY at the emit site (intersected into the widen
// only when the child component uses `$$props` / `$$restProps`). Mirror
// of upstream's `SvelteAllProps` (svelte-shims-v4.d.ts:39), which
// upstream applies via `__sveltets_2_with_any(…)` or
// `__sveltets_2_partial_with_any(…)` factory functions when the child's
// `uses$$props` flag is set. Components that DON'T reference those
// identifiers keep strict Props — matching upstream's TS2353 on
// undeclared attrs.
declare type __SvnAllProps = { [index: string]: any };
// `children?: any` mirrors upstream's `__sveltets_2_PropsWithChildren`
// widen (svelte-shims-v4.d.ts:258-266) — lets the consumer-side
// implicit-children emission (`children: () => __svn_snippet_return()`
// on `<Foo>body</Foo>` patterns) type-check against Svelte 4
// components that have `<slot>` usage. Previously we included a
// catch-all `{ [index: string]: any }` which accepted `children` but
// also contaminated tsgo's assignability check — TS2322 top-level
// error fired instead of the precise TS2741 on missing required props
// (observed on language-tools/.../test-error/Index.svelte's
// `<Jsdoc />` vs expected TS2741). Dropping the index sig requires
// users of Svelte 4 components to not pass undeclared attrs — same
// strictness as upstream.

// SVELTE-4-COMPAT: `$$Generic<T>` is Svelte 4's pre-Svelte-5-generics-attr
// syntax for declaring a generic type parameter on a component — written
// as `type T = $$Generic<any>`. The syntax has no Svelte 5 equivalent;
// we alias to `any` so the reference resolves and the user's type usage
// downstream type-checks (loosely).
declare type $$Generic<T = any> = T;

// SVELTE-4-COMPAT: `__svn_invalidate(() => expr)` wraps the RHS of a
// reactive declaration (`$: NAME = expr`) in a lazy thunk. The
// purpose is purely type-checking: the thunk body is NEVER invoked,
// so TS's control-flow analysis treats any identifier references
// inside as lazy. That matters when `expr` references a `const`
// function declared LATER in the script — e.g.:
//
//     $: foo = helper(x)
//     const helper = (x: X) => …
//
// Without the wrap, TS fires TS2448 "used before its declaration"
// on `helper` because `$: foo = …` becomes `let foo = helper(x)`
// at source position, and the `const helper` at a later position
// triggers TDZ. With the wrap (`let foo = __svn_invalidate(() =>
// helper(x))`), the reference is inside an uncalled arrow; TDZ
// analysis doesn't apply, and the return type still flows out as
// the inferred `T` of the thunk.
//
// Mirrors upstream svelte2tsx's `__sveltets_2_invalidate` helper.
declare function __svn_invalidate<T>(fn: () => T): T;

// @@STATE_AMBIENTS_BEGIN@@
// `$state<T>` ambient declarations. Stripped when real svelte 5 is
// installed — Svelte's `types/index.d.ts:3221-3222` declares the
// same two overloads. Keeping both sets produces 4 identical
// overloads, which poisons TS's overload resolution: a mismatch on
// `$state<T>(initial: T)` reports TS2769 "No overload matches this
// call" instead of the expected TS2741 "Property 'X' is missing in
// type Y" that fires with 2 overloads. Minimal repro at
// test_dup_overload.ts confirmed. Other rune ambients ($derived,
// $effect, etc.) aren't stripped because either their single-overload
// form is immune to the dedup issue or our shim carries extra
// overloads (e.g. `$props<T = any>()`) that Svelte's simpler
// declarations don't provide — stripping those would fire TS2558 on
// user-authored `$props<MyShape>()` calls.
/** `$state<T>(initial?)` declares reactive state. Macro.
 *
 * Two overloads:
 *   - `$state(value)` — normal initial. T inferred from the argument
 *     (or the explicit generic when one is given).
 *   - `$state()` — no initial. Return is `T | undefined`.
 *
 * Calls like `$state<T>(0)` where T is a generic parameter and 0 isn't
 * assignable to T fire TS2345 — matches Svelte's own behavior.
 *
 * Historical note: we previously had two additional overloads for
 * `initial: null` and `initial: undefined` literal types, there to
 * preserve `T` against the variable's annotation in the bind:this
 * pattern (`let el: HTMLInputElement | null = $state(null)`). Those
 * overloads collide with TypeScript's overload resolution on
 * `$state<Promise<T>>(new Promise(() => {}))`: when a generic
 * function's overload set includes literal-type parameters, the
 * explicit `<T>` argument no longer propagates as contextual type to
 * the call's argument. The inner `new Promise(() => {})` then widens
 * to `Promise<unknown>` and no overload matches — TS2769. This is
 * TypeScript behavior across both tsc and tsgo, not a tsgo-only gap.
 * The emit crate now rewrites
 * `let X: Type = $state(null | undefined)` to
 * `let X: Type = $state<Type>(null | undefined)` (see
 * `state_nullish_rewrite`), which lets this single-T shim handle
 * both the bind:this pattern and the `$state<Promise<T>>(...)`
 * pattern without conflict.
 */
declare function $state<T>(initial: T): T;
declare function $state<T>(): T | undefined;
// @@STATE_AMBIENTS_END@@
declare namespace $state {
    function eager<T>(value: T): T;
    function raw<T>(initial: null): T;
    function raw<T>(initial: undefined): T;
    function raw<T>(initial: T): T;
    function raw<T>(): T | undefined;
    function snapshot<T>(value: T): T;
}

/** `$derived(expression)` re-evaluates whenever its dependencies change. */
declare function $derived<T>(expression: T): T;
declare namespace $derived {
    function by<T>(fn: () => T): T;
}

/** `$effect(fn)` runs a side effect after every dependency change. */
declare function $effect(fn: () => void | (() => void)): void;
declare namespace $effect {
    function pre(fn: () => void | (() => void)): void;
    function root(fn: () => void | (() => void)): () => void;
    function tracking(): boolean;
    function pending(): number;
}

/** `$props<T>()` declares the component's prop bag. */
declare function $props<T = Record<string, any>>(): T;
declare namespace $props {
    function id(): string;
}

/** `$bindable<T>(fallback?)` marks a prop as two-way bindable. */
declare function $bindable<T>(fallback?: T): T;

/** `$inspect(...values)` logs values whenever they change in dev.
 *
 * `with` is declared as a property (arrow function type), not a method.
 * Matches real svelte: the property form is contravariant in its
 * parameter type, which matters when the returned object is assigned
 * to a stricter handler shape — method form would be bivariant and
 * silently accept looser callbacks.
 */
declare function $inspect<T extends any[]>(
    ...values: T
): { with: (fn: (type: 'init' | 'update', ...values: T) => void) => void };
declare namespace $inspect {
    function trace(name?: string): void;
}

/** `$host<El>()` returns the host element for a custom-element component.
 *
 * Constraint matches real svelte: the parameter must extend `HTMLElement`.
 */
declare function $host<El extends HTMLElement = HTMLElement>(): El;

// Internal helpers emitted by svelte-check-native into generated `.svelte.ts`
// files. Declared here so the generated code type-checks. The `__svn_*`
// prefix marks them as ours; user code shouldn't touch them.

/**
 * Iterable wrapper for `{#each}` blocks. Mirrors upstream's
 * `__sveltets_2_ensureArray<T extends ArrayLike<unknown> |
 * Iterable<unknown>>(array: T | undefined | null)`
 * (`svelte-shims-v4.d.ts:253-256`). The constraint fires TS2345 on
 * non-arraylike non-iterable expressions (`{#each {}}`,
 * `{#each 1}`); the `T | undefined | null` parameter widening lets
 * `Foo[] | undefined` narrow to `Foo[]` for item typing without
 * losing the runtime null-tolerance.
 */
declare function __svn_each_items<T extends ArrayLike<unknown> | Iterable<unknown>>(
    value: T | undefined | null,
): Iterable<__SvnEachItem<T>>;

/** Resolved item type for `__svn_each_items`. The `0 extends 1 & T` guard preserves `any` (avoids the conditional-type-distribution-collapses-to-unknown trap). */
type __SvnEachItem<T> = 0 extends 1 & T
    ? any
    : T extends ArrayLike<infer U>
        ? U
        : T extends Iterable<infer U>
            ? U
            : never;

/**
 * Reviewer follow-up #2: extract a child component's events surface
 * for the parent's bubbled-event projection. When the wrapper has
 * `<Child on:NAME />` (no value, event-bubble shorthand), the
 * wrapper's own `$$Events` carries NAME with Child's declared event
 * type — projected as `__SvnComponentEvents<typeof Child>["NAME"]`.
 *
 * Three branches:
 *   1. `__svn_events` marker present (iso shape with declared
 *      $$Events) → return the marker's E.
 *   2. Plain `Component<P, X, B>` (fn-component shape, no events
 *      surface) → return `Record<string, any>` so the projected
 *      event types as `any` (matches upstream's lax fallback for
 *      runes-only components).
 *   3. Anything else (synthetic dynamic-component root, malformed
 *      input) → `Record<string, any>` lax fallback.
 *
 * Mirrors upstream svelte2tsx's `__sveltets_2_bubbleEventDef(
 * __sveltets_2_instanceOf(<Comp>).$$events_def, '<name>')`
 * semantics at type-level — we project from the typed marker
 * directly, no runtime helper call needed.
 */
/**
 * Reviewer follow-up #3 (round 4): also extract events from
 * legacy / external `SvelteComponentTyped<Props, Events, Slots>`
 * class constructors that don't carry the `__svn_events` marker.
 * Pre-fix only the marker branch fired, so package-installed
 * Svelte-3-style components forwarded their events as `any` when
 * a wrapper bubbled them.
 *
 * Branch order:
 *   1. `__svn_events` marker — our overlay's strict shape.
 *   2. Class constructor returning a `SvelteComponent<P, E, S>`
 *      instance — covers both legacy `SvelteComponentTyped<P, E,
 *      S>` (deprecated alias) and our own iso interface's `new`
 *      sig when the marker is absent. Inferring `E` directly
 *      from `SvelteComponent`'s second type parameter.
 *   3. Callable `Component<P, X, B>` (Svelte-5 fn-component
 *      shape) — no events parameter. Fall through to
 *      `Record<string, any>` for the lax fallback (matches
 *      upstream's behavior for runes-only components).
 */
type __SvnComponentEvents<C> = C extends { readonly __svn_events: infer E }
    ? E
    : C extends new (...args: any[]) => import('svelte').SvelteComponent<any, infer E extends Record<string, any>, any>
      ? E
      : Record<string, any>;

/**
 * SlotHandler PLAN Stage 4: extract a child component's slot
 * surface for the parent's let-forwarded slot projection. When
 * a wrapper has `<Wrapper let:tooltip><slot {tooltip}/></Wrapper>`,
 * the slot-def's `tooltip` entry projects as
 * `__SvnComponentSlots<typeof Wrapper>['default']['tooltip']`.
 *
 * Branch order:
 *   1. `__svn_slots` marker — reserved for a future strict-shape
 *      opt-in (no current emit path produces it).
 *   2. Class constructor returning a `SvelteComponent<P, E, S>`
 *      instance — extracts `S` directly. Covers our iso interface's
 *      `new` signature AND legacy `SvelteComponentTyped<P, E, S>`
 *      class components.
 *   3. Anything else (callable `Component<P, X, B>` /
 *      synthetic root) → `Record<string, Record<string, any>>` so
 *      `[slotName][propName]` indexing falls through to `any`
 *      without a TS lookup error.
 */
type __SvnComponentSlots<C> = C extends { readonly __svn_slots: infer S }
    ? S
    : C extends new (...args: any[]) => import('svelte').SvelteComponent<any, any, infer S extends Record<string, any>>
      ? S
      : Record<string, Record<string, any>>;

/**
 * Reviewer follow-up #3b: convert a wrapped `$$Events` map back to
 * the DETAIL form for `createEventDispatcher`'s type argument. The
 * wrapped form `{ name: CustomEvent<T> }` is what the user declares
 * in `interface $$Events`; the dispatcher's `<T>` wants the inner
 * detail type for each entry, so unwrap each `CustomEvent<…>`
 * back to `…`.
 *
 * Mirrors upstream `__sveltets_2_CustomEvents` in
 * `svelte2tsx/svelte-shims.d.ts`. Used in the synthesised
 * `createEventDispatcher<__SvnCustomEvents<$$Events>>()` rewrite —
 * after the rewrite, `dispatch('name', detail)` calls type-check
 * `detail` against the original `$$Events.name` payload type.
 */
/**
 * Reviewer follow-up #4 (round 4): filter to keys whose declared
 * value is `CustomEvent<…>` so non-CustomEvent declared events
 * don't leak into the dispatcher's signature. Pre-fix the helper
 * kept every key of `$$Events` and merely unwrapped CustomEvent —
 * so an `interface $$Events { click: MouseEvent }` would
 * incorrectly permit `dispatch('click', …)` on an event that's
 * actually a native DOM event (which Svelte's runtime can't
 * dispatch via `createEventDispatcher`).
 *
 * Mirrors upstream `__sveltets_2_CustomEvents` byte-for-byte
 * (`svelte-shims.d.ts:139-141`): `KeysMatching<T, CustomEvent>`
 * narrows to dispatchable entries; the inner conditional
 * unwraps the detail type per entry.
 */
type __SvnKeysMatching<Obj, V> = {
    [K in keyof Obj]-?: Obj[K] extends V ? K : never;
}[keyof Obj];

type __SvnCustomEvents<T> = {
    [K in __SvnKeysMatching<T, CustomEvent>]: T[K] extends CustomEvent ? T[K]['detail'] : T[K];
};

/**
 * Fresh `any` placeholder. Used as the anchor / target argument in the
 * emitted `new Comp({ target: __svn_any(), props: {...} })` call.
 *
 * Declared generic with `T = any` so callers can narrow the return at
 * the call site when needed; default usage gets plain `any`.
 */
declare function __svn_any<T = any>(): T;

/**
 * `<svelte:self>` synthetic — the file's own component default
 * referenced from inside its own template. We can't easily get
 * "the component's own props" inside its own render fn (circular
 * dep), so type as `any`-component: the `new __svn_C({…})` call
 * goes through `__svn_ensure_component(__svn_self_default)` which
 * returns an `any`-prop ctor. Excess-prop checks degenerate to
 * "any prop accepted" but the rest of the component (events,
 * bindings, children) still type-checks via the normal path.
 *
 * Mirrors upstream svelte2tsx's `__sveltets_2_createComponentAny`
 * for `<svelte:self>` (see InlineComponent.ts:99).
 */
declare const __svn_self_default: import('svelte').Component<any, any, any>;

/**
 * JS-overlay definite-assign: `let b; b = __svn_any(b);` is the JS
 * equivalent of the TS-overlay `let b!: T;` splice — a self-assign
 * through an any-cast helper that satisfies TS flow analysis without
 * emitting TS-only syntax (`!:`, `as`) that would fire TS8010 in a
 * `.svelte.svn.js` file. Mirrors upstream svelte2tsx's
 * `__sveltets_2_any(name)` self-assignment pattern (see
 * ExportedNames.ts; produces `b = __sveltets_2_any(b)` after each
 * Svelte-4 `export let` declaration).
 *
 * Return type is `any` unconditionally — the purpose is to widen,
 * not preserve, so downstream reads aren't flow-narrowed back to
 * the original (possibly uninitialised-shaped) type.
 */
declare function __svn_any(x: any): any;

/**
 * `interface $$Props` cross-check shim. When the component declares a
 * Svelte-4 `$$Props` interface AND a sibling `export let X: T`, the
 * render fn returns its props as
 *
 *     { ...__svn_ensure_right_props<{ X: T; ... }>(__svn_any("") as $$Props) } as $$Props
 *
 * (mirrors upstream svelte2tsx's `__sveltets_2_ensureRightProps` —
 * `svelte-shims-v4.d.ts:62`). The type-arg constraint fires TS2345
 * when `$$Props['X']` is wider than `T` (optional vs required) or
 * missing a let-declared name. The `: {}` return is intentionally
 * empty so the spread leaves the surrounding `as $$Props` cast as
 * the props' final type — no inference leak from the assertion.
 */
declare function __svn_ensure_right_props<Props>(props: Props): {};

/**
 * Svelte 5 `bind:X={getter, setter}` helper. Mirrors upstream
 * `__sveltets_2_get_set_binding` (svelte2tsx/svelte-shims-v4.d.ts:269)
 * with the `__svn_*` prefix mandated by CLAUDE.md architecture rule #6.
 *
 * `T` is inferred once per call site. The getter's return and the
 * setter's parameter are BOTH checked against `T`, and the return
 * flows to the prop slot — so a mismatched setter (e.g. `bind:value={
 * () => s, (n: number) => …}` where the child expects `string`) fires
 * TS2322/TS2345 at the call site. Without this helper, emit would
 * invoke just the getter (`(getter)()`) and the setter would go
 * type-unchecked.
 */
declare function __svn_get_set_binding<T>(
    get: (() => T) | null | undefined,
    set: (t: T) => void,
): T;

/**
 * Normalize any component shape to a constructible so one emission
 * works uniformly across the shapes a real Svelte codebase mixes:
 *
 *   - Svelte 5 callable (our overlay defaults, bare `Component<Props>`
 *     values from user-typed contexts) — wrapped in a synthesized
 *     construct signature whose props slot carries the original Props.
 *   - Svelte-4-style class (lucide-svelte, phosphor-svelte, bits-ui,
 *     any `extends SvelteComponent` export) — passthrough; the class
 *     already is constructible, and its generic parameters stay on
 *     the return type so `new $$_C<T>(...)` infers T from props.
 *
 * Per-call-site emission form:
 *
 *     { const $$_CN = __svn_ensure_component(Comp);
 *       new $$_CN({ target: __svn_any(), props: { ... } }); }
 *
 * The intermediate local is what makes generic inference work: TS
 * binds the construct signature's generics at the `new` site (seeing
 * the concrete prop values) rather than at the `__svn_ensure_component`
 * site (where only the component type is visible). Dropping the local
 * collapses `T` to `unknown` for generic components, firing
 * implicit-any on snippet arrows over the generic.
 *
 * Overload order matters: TS picks the first match. `Component<P>` has
 * to come before the generic `(anchor, props: P)` overload — a value
 * typed `Component<P>` structurally matches `(anchor, props)` too
 * (both have call signatures), and matching the latter first binds
 * P to `any` and kills contextual typing. Component first forces TS
 * to read P out of the Component's generic slot.
 *
 * `props?: Partial<P>` on the synthesized constructor keeps required
 * props optional at the `new $$_C({...})` call site — real components
 * routinely receive props via bind: directives, spreads, or implicit
 * `children` snippets (none of which show up in our emitted object
 * literal). Partial preserves the excess-property check (typo'd prop
 * names still fire TS2353) and contextual-typing flow (callback
 * destructures, snippet params).
 */
// SVELTE-4-COMPAT — v0.3 Item 3: typed-events overload.
//
// When a child component declares `interface $$Events { ... }` (or
// `type $$Events = ...`), the child's emit intersects its default
// export with `& { readonly __svn_events: $$Events }`. That property
// presence is what this overload keys on: it matches ONLY typed
// children, binds E out of `__svn_events`, and returns an
// `__SvnInstanceTyped<P, E>` whose `$on<K extends keyof E>` narrows
// handler signatures per declared event.
//
// Untyped children (no `$$Events` declaration — the common case
// including all Svelte-5 runes-mode children) fall through to the
// lax overload below and get `__SvnInstance<P>` whose `$on(event:
// string, handler: (...args: any[]) => any)` contextually types
// destructures like `({detail}) => …` to `any` — critical to avoid
// TS7031 at workspace scale (the regression that sunk the reverted
// conditional-dispatch attempt in v0.2.5).
//
// Overload order MATTERS: typed must come first so it's preferred
// when the intersection is present. Validated end-to-end via
// /tmp/svn-item3-fixture/real_component.ts.
//
// The Component-arm uses conditional-type distribution (via
// `T extends … ? … : never`) instead of a plain generic binding
// `<P extends Record<string, any>>(c: Component<P, any, any>)`. When
// the input is a UNION of `Component<P1> | Component<P2> | …`
// (the dynamic-component pattern
// `{@const X = fieldType.component}` seen on a CMS-style bench),
// the conditional distributes: each union member produces
// its own ctor type, and the union of ctors intersects their
// contravariant arg positions — the resulting `options.props?` slot
// becomes `P1 & P2 & … & Pn`. Consumer prop literals must satisfy
// that intersection (TS2322 on structural mismatches), matching
// upstream svelte2tsx byte-for-byte on PageFieldField.svelte /
// SiteField.svelte.
//
// Without the conditional, TS's overload resolver falls through to
// the `c: unknown` fallback when T is a union, giving `props?: any`
// and silently accepting any prop literal.
// 2026-04-25: unified with upstream's single-overload conditional-
// return pattern (svelte-shims-v4.d.ts:224-251). Produces instance
// shapes identical to what the default-export emit's
// `$$IsomorphicComponent` pattern yields — so consumer-side
// `bind:this={ref}` against a user-declared `let ref: MyComp`
// target matches structurally.
//
// Branch order matters. TS tries each conditional in turn:
//
// 1. Typed-events marker: the child's emit intersects
//    `& { readonly __svn_events: $$Events }` onto its default-export
//    VALUE when `interface $$Events` / `type $$Events` is declared.
//    We match that first so narrowed event-handler typing fires
//    even when the input ALSO has a `new` signature (which
//    `$$IsomorphicComponent` does). The events shape is wrapped in
//    `CustomEvent<>` here so user handlers written as
//    `(e: CustomEvent<{id:number}>) => …` match
//    `SvelteComponent.$on<K>(cb: (e: Events[K]) => void)` where
//    Events[K] = CustomEvent<{id:number}>.
//
// 2. Constructor passthrough: Svelte-4 legacy class components
//    (extending `SvelteComponent` directly) pass through unchanged
//    — `new C({…})` on the returned type already produces the
//    right `InstanceType<C>` shape.
//
// 3. `Component<P, Exports, Bindings>` — Svelte-5 component shape
//    without typed-events marker. Returns a ctor whose instance
//    matches what the default-export's `$$IsomorphicComponent` new
//    signature yields.
//
// 4. Function-component fallback: `(anchor, props: P) => any`
//    shape. Mostly covers user-authored raw functions; rare.
//
// 5. Fallback `any`-prop ctor for unknown shapes (union types, etc.)
//    that fall through the above.
declare function __svn_ensure_component<
    T extends
        | (new (...args: any[]) => any)
        | import('svelte').Component<any, any, any>
        | ((anchor: any, props: any) => any)
        | { readonly __svn_events: any }
        | null
        | undefined,
>(
    c: T,
): NonNullable<
    T extends { readonly __svn_events: infer E extends Record<string, any> } & import('svelte').Component<
        infer P extends Record<string, any>,
        infer X extends Record<string, any>,
        infer B extends string
    >
        ? new (options: { target?: any; props?: P }) => import('svelte').SvelteComponent<
              P,
              E,
              Record<string, any>
          > &
              X & { $$bindings?: B }
        : T extends new (...args: any[]) => any
          ? T
          : T extends import('svelte').Component<
                  infer P extends Record<string, any>,
                  infer X extends Record<string, any>,
                  infer B extends string
              >
            ? new (options: { target?: any; props?: P }) => import('svelte').SvelteComponent<
                  P,
                  Record<string, any>,
                  Record<string, any>
              > &
                  X & { $$bindings?: B }
            : T extends (anchor: any, props: infer P) => any
              ? P extends Record<string, any>
                  ? new (options: {
                        target?: any;
                        props?: P;
                    }) => import('svelte').SvelteComponent<P>
                  : new (options: {
                        target?: any;
                        props?: any;
                    }) => import('svelte').SvelteComponent<Record<string, any>>
              : new (options: {
                    target?: any;
                    props?: any;
                }) => import('svelte').SvelteComponent<Record<string, any>>
>;

/**
 * Shape returned by a `new __svn_ensure_component(C)({target, props})`
 * call. `$$prop_def` is the compile-time-only carrier used elsewhere
 * in the shim chain; `$on` accepts the SVELTE-4-COMPAT
 * `$inst.$on("event", handler)` pattern the emit uses for `on:event`
 * directives on components.
 *
 * `handler` is typed as a callable `(...args: any[]) => any` rather
 * than bare `any` so the arrow function the user passes gets
 * contextual typing from the callable shape. With bare `any`, the
 * arrow's `({detail}) => ...` parameter destructure falls back to
 * TS's fresh inference — no context — and fires TS7031 under
 * `noImplicitAny`. The callable form pushes `any` into each
 * positional param, which is what makes the destructure fine.
 */
type __SvnInstance<P> = {
    $$prop_def: P;
    $on(event: string, handler: (...args: any[]) => any): () => void;
};

/**
 * SVELTE-4-COMPAT — v0.3 Item 3. Typed-events counterpart to
 * `__SvnInstance<P>`. `$on` dispatches against the declared events
 * map `E`: the event name must be `keyof E`, and the handler sees a
 * `CustomEvent<E[K]>` with the declared payload — so `e.detail`
 * narrows to the right shape in the handler body.
 *
 * Selected by the typed overload of `__svn_ensure_component` when
 * the child component's default export carries
 * `{ readonly __svn_events: E }` (emit intersects this in when a
 * `$$Events` interface/type is declared in the child). For children
 * without that marker, `__SvnInstance<P>` is selected instead and
 * `$on` stays lax.
 *
 * Mirrors upstream svelte2tsx's `hasStrictEvents`-branching shape
 * (see `_events(strictEvents, renderStr)` in
 * `language-tools/packages/svelte2tsx/src/svelte2tsx/addComponentExport.ts`).
 * Upstream's non-strict branch INTERSECTS Events with `{[evt:
 * string]: CustomEvent<any>}`; ours does the same implicitly by
 * selecting the lax `__SvnInstance<P>` at the overload level
 * instead of typing it up through an intersection. Equivalent
 * observed semantics, simpler shim.
 */
type __SvnInstanceTyped<P, E> = {
    $$prop_def: P;
    // E carries the FINAL `$on` event-object map (matches upstream
    // convention: user's `interface $$Events { click: MouseEvent }`
    // means `$on('click', cb)`'s cb is `(e: MouseEvent)`. If user
    // wants `CustomEvent<…>`, they wrap explicitly in their
    // interface). The synthesized typed-dispatcher case is wrapped
    // ONCE at synthesis (`type $$Events = { [K]: CustomEvent<T[K]> }`)
    // so this type is final regardless of source.
    $on<K extends keyof E>(event: K, handler: (e: E[K]) => any): () => void;
};

/**
 * Partial<> variant that widens each prop with `| null`. Required
 * props become optional (same as `Partial<>` — bind:, spread, and
 * implicit children absorb the "missing" case), AND variables the
 * user typed `T | null` (common with `bind:this` stored in `$state<T
 * | null>(null)`) can be passed in without a TS2322 "`HTMLElement |
 * null` not assignable to `HTMLElement | undefined`" mismatch.
 * Excess-property checks (typo'd prop names) and contextual-typing
 * flow (callback destructures, snippet params) are preserved.
 */
type __SvnPropsPartial<P> = { [K in keyof P]?: P[K] | null };

// v0.3 Item 7: the `__svn_bind_this_check<El>(target)` shim that
// previously lived here was removed. Its `target: El | null |
// undefined` signature rejected legitimate broader-type declarations
// (`let el: HTMLElement | null` on a `<div>`) because `HTMLElement`
// is a SUPERTYPE of `HTMLDivElement`. Current Item 7 emit uses the
// assignment-direction shape (matches upstream's Binding.ts:85-93):
//     void /* bind:this */ ((): void => {
//         EXPR = null as any as HTMLElementTagNameMap['tag'];
//     });
// where the LHS-accepts-RHS check correctly admits broader
// declared types while still flagging truly-wrong element types
// (e.g. `HTMLSpanElement` declared, bound on `<input>`).

/**
 * Phantom type-compatibility check for one-way-not-on-element DOM
 * bindings. Used in the template-check body for directives like
 * `bind:contentRect={rect}` / `bind:buffered={buf}` where the runtime
 * type lives on a separate browser API (ResizeObserver for
 * content-rect, HTMLMediaElement SvelteMediaTimeRange for buffered).
 *
 * Called as `__svn_any_as<DOMRectReadOnly>(rect);`. The single
 * argument being typed `T` means `rect`'s declared type must accept
 * `T` — TS2322 fires on `let rect: string; __svn_any_as<DOMRectReadOnly>(rect);`.
 * No return, no side effect, no mutation to `rect`'s inferred type:
 * the call vanishes under `void`-free evaluation, and TS flow
 * analysis sees only a "read rect" followed by no narrowing.
 */
declare function __svn_any_as<T>(value: T): void;

/**
 * Branded-`any` return for snippet arrow-callback bodies. Svelte's
 * `Snippet<[...]>` type brands its return shape so a bare
 * `(args) => void` can't structurally satisfy it. The arrow emits a
 * `return __svn_snippet_return();` tail so the callback assigns
 * cleanly into a `Snippet<[...]>` prop slot while contextual typing
 * still flows from the slot's signature into the parameters.
 */
declare function __svn_snippet_return(): any;

/**
 * CSS-custom-property prop on a component — Svelte 5 accepts
 * `<Foo --accent-color="red">` as a CSS variable passthrough to
 * the component's wrapper, NOT as a typed prop. Emit spreads the
 * value through this helper so the key contributes `{}` (nothing)
 * to the component's Props object — no TS2353 "does not exist in
 * type" against the component's declared Props. Mirrors upstream
 * svelte2tsx's `__sveltets_2_cssProp`.
 */
declare function __svn_css_prop(prop: Record<string, any>): {};

/**
 * Action-directive return shape — matches Svelte's `ActionReturn` plus
 * the `$$_attributes` hook svelte2tsx uses to forward action-declared
 * attributes back onto the element.
 */
type __SvnActionReturnType =
    | {
          update?: (args: any) => void;
          destroy?: () => void;
          $$_attributes?: Record<string, any>;
      }
    | void;

/**
 * Wraps an action invocation — `action(element, params)` — so its
 * return value type-checks against `ActionReturn` and any
 * `$$_attributes` the action advertises can be picked up by the
 * enclosing element's attribute pass.
 *
 * The important half for us is the ARGUMENT side: `action(element,
 * params)` is a real function call, so TypeScript contextually types
 * `params` against the action's declared second parameter. For
 * `use:enhance={({formData}) => ...}` that flows `SubmitFunction`'s
 * parameter shape into the arrow's destructure — and fires TS2339 on
 * any property name that isn't on that shape (the user-reported
 * `{form, data, submit}` miss).
 */
declare function __svn_ensure_action<T extends __SvnActionReturnType>(
    actionCall: T,
): T extends { $$_attributes?: any } ? T['$$_attributes'] : {};

/**
 * Transition-directive return shape — matches Svelte's
 * `TransitionConfig` (or a thunk producing one). Mirrors upstream's
 * `__sveltets_2_SvelteTransitionReturnType` at
 * `language-tools/packages/svelte2tsx/svelte-shims-v4.d.ts:175-176`.
 */
type __SvnTransitionConfig = {
    delay?: number;
    duration?: number;
    easing?: (t: number) => number;
    css?: (t: number, u: number) => string;
    tick?: (t: number, u: number) => void;
};
type __SvnTransitionReturnType = __SvnTransitionConfig | (() => __SvnTransitionConfig);

/**
 * Wraps a `transition:` / `in:` / `out:` directive invocation —
 * `transitionFn(element, params)` — so its return value type-checks
 * against `TransitionConfig`. The wrapper is also the syntactic
 * anchor the diagnostic post-filter
 * (`crates/typecheck/src/filters.rs::is_overlay_in_ensure_transition_call`)
 * uses to drop TS2554 "Expected 3 arguments" — Svelte's transition
 * runtime supplies the optional 3rd `_context` parameter, but tsgo
 * fires 2554 when the user's transition function declares it as
 * required and we only pass 2 args at the synthetic call site. Mirrors
 * upstream's `__sveltets_2_ensureTransition` + the
 * `expectedTransitionThirdArgument` filter at
 * `language-server/src/plugins/typescript/features/DiagnosticsProvider.ts:663-700`.
 */
declare function __svn_ensure_transition(transitionCall: __SvnTransitionReturnType): {};

/**
 * Intersect up to N action-return-attributes types so they flow
 * through `svelteHTML.createElement("tag", actions, attrs)`'s 3-arg
 * overload. Upstream `svelte2tsx` emits this as `__sveltets_2_union`;
 * the signature is the same — return type is `T1 & T2 & T3 & …`.
 *
 * Called as `__svn_union(__svn_action_0, __svn_action_1, …)` when an
 * element has `use:` directives. The intersection is the second arg
 * to `svelteHTML.createElement` (the `attrsEnhancers: T` slot); the
 * attrs literal's type becomes `Elements[Key] & T` which tsgo
 * eagerly expands (unlike the 2-arg overload's `Elements[Key]` alias
 * form). This gives TS2353 diagnostic messages against the expanded
 * `Omit<HTMLAttributes<HTMLDivElement>, never> & HTMLAttributes<any>`
 * form that matches upstream byte-for-byte.
 */
declare function __svn_union<T1, T2, T3, T4, T5, T6, T7, T8, T9, T10>(
    t1: T1,
    t2?: T2,
    t3?: T3,
    t4?: T4,
    t5?: T5,
    t6?: T6,
    t7?: T7,
    t8?: T8,
    t9?: T9,
    t10?: T10,
): T1 & T2 & T3 & T4 & T5 & T6 & T7 & T8 & T9 & T10;

/**
 * Map an HTML/SVG tag name back to the real element type so action
 * directives emit `action(__svn_map_element_tag('form'), params)` with
 * a proper `HTMLFormElement` in the first slot rather than `unknown`
 * or `any`. Actions that declare a specific element type (e.g.
 * `Action<HTMLFormElement, P>`) will TS2345 against the concrete type
 * if the tag doesn't match.
 *
 * Unknown tags fall through to `HTMLElement` — matching upstream
 * svelte2tsx's `svelteHTML.mapElementTag` behavior.
 */
declare function __svn_map_element_tag<K extends keyof HTMLElementTagNameMap>(
    tag: K,
): HTMLElementTagNameMap[K];
declare function __svn_map_element_tag<K extends keyof SVGElementTagNameMap>(
    tag: K,
): SVGElementTagNameMap[K];
declare function __svn_map_element_tag(tag: string): HTMLElement;

/**
 * Phantom value used as the second argument to animate-directive call
 * emissions. Svelte's `Animation` typing declares
 *
 *     (node: Element, animation: { from: DOMRect; to: DOMRect }, params?: P) => AnimationConfig
 *
 * — the middle slot has a fixed structural shape we don't synthesize
 * at the call site. Mirrors upstream svelte2tsx's
 * `__sveltets_2_AnimationMove`. See the `animate:` directive emit
 * (`crates/emit/src/nodes/animation.rs`).
 */
declare const __svn_AnimationMove: { from: DOMRect; to: DOMRect };

/**
 * Validate that a style-directive value expression type-checks
 * against the set of legal CSS-value runtime types. Emitted for
 * each `style:prop={value}` as
 *   `__svn_ensure_type(String, Number, value);`
 * and for each text+mustache quoted form as
 *   `__svn_ensure_type(String, Number, \`…${expr}…\`);`
 *
 * The single-type form accepts `T | undefined | null`; the two-type
 * form accepts `T1 | T2 | undefined | null`. Passing an `unknown`
 * binding fires TS2345 "Argument of type 'unknown' is not
 * assignable…", mirroring upstream svelte2tsx's
 * `__sveltets_2_ensureType` behavior
 * (`language-tools/packages/svelte2tsx/svelte-shims-v4.d.ts:180-181`).
 *
 * Historical note: previously the 3rd param was loose `unknown`.
 * That was a workaround for charting-lib-style Canvas/Html/Svg
 * TS7034/7005 false-positives on Svelte-4 `export let zIndex =
 * undefined` props. The JS-overlay flip (`.svelte.svn.js` for
 * lang=js sources) routes those files through `noImplicitAny:false`,
 * so the strict constraint is safe now. The stricter form is
 * load-bearing for CMS-style component-preview / style-directive
 * TS2345/TS18046 diagnostics.
 */
declare function __svn_ensure_type<T>(
    type: new (...args: any[]) => T,
    el: T | undefined | null,
): {};
declare function __svn_ensure_type<T1, T2>(
    type1: new (...args: any[]) => T1,
    type2: new (...args: any[]) => T2,
    el: T1 | T2 | undefined | null,
): {};

// Ambient `svelteHTML` namespace — VENDORED VERBATIM from upstream
// language-tools/packages/svelte2tsx/svelte-jsx-v4.d.ts (MIT-licensed).
// Mirrors upstream svelte-check's bundled `svelte-jsx-v4.d.ts` so the
// DOM-element emit (`svelteHTML.createElement("tag", { …attrs })`)
// resolves with full per-element attribute typing.
//
// Why vendor instead of referencing user's `svelte/svelte-html.d.ts`:
// svelte's package.json doesn't expose svelte-html.d.ts through its
// `exports` map (by design — "deliberately not exposed through the
// exports map" per its header). Upstream `svelte-check` vendors this
// same file; we follow suit.
//
// Per-element attribute types resolve through
// `import('svelte/elements').SvelteHTMLElements[K]`. When the user has
// svelte installed, real attribute catalogs flow through (full
// per-element typing: `button.type: "button"|"reset"|"submit"|...`).
// Without svelte, our fallback resolves `HTMLAttributes<T> = any` and
// the check degrades gracefully.

declare namespace svelteHTML {
    function mapElementTag<K extends keyof ElementTagNameMap>(
        tag: K
    ): ElementTagNameMap[K];
    function mapElementTag<K extends keyof SVGElementTagNameMap>(
        tag: K
    ): SVGElementTagNameMap[K];
    function mapElementTag(tag: any): any;

    function createElement<Elements extends IntrinsicElements, Key extends keyof Elements>(
        element: Key | undefined | null,
        attrs: string extends Key ? import('svelte/elements').HTMLAttributes<any> : Elements[Key]
    ): Key extends keyof ElementTagNameMap
        ? ElementTagNameMap[Key]
        : Key extends keyof SVGElementTagNameMap
            ? SVGElementTagNameMap[Key]
            : any;
    function createElement<Elements extends IntrinsicElements, Key extends keyof Elements, T>(
        element: Key | undefined | null,
        attrsEnhancers: T,
        attrs: (string extends Key ? import('svelte/elements').HTMLAttributes<any> : Elements[Key]) & T
    ): Key extends keyof ElementTagNameMap
        ? ElementTagNameMap[Key]
        : Key extends keyof SVGElementTagNameMap
            ? SVGElementTagNameMap[Key]
            : any;

    interface HTMLAttributes<T extends EventTarget = any> {}
    interface SVGAttributes<T extends EventTarget = any> {}

    type HTMLProps<Property extends keyof import('svelte/elements').SvelteHTMLElements, Override> =
        Omit<import('svelte/elements').SvelteHTMLElements[Property], keyof Override> & Override;

    interface IntrinsicElements {
        a: HTMLProps<'a', HTMLAttributes>;
        abbr: HTMLProps<'abbr', HTMLAttributes>;
        address: HTMLProps<'address', HTMLAttributes>;
        area: HTMLProps<'area', HTMLAttributes>;
        article: HTMLProps<'article', HTMLAttributes>;
        aside: HTMLProps<'aside', HTMLAttributes>;
        audio: HTMLProps<'audio', HTMLAttributes>;
        b: HTMLProps<'b', HTMLAttributes>;
        base: HTMLProps<'base', HTMLAttributes>;
        bdi: HTMLProps<'bdi', HTMLAttributes>;
        bdo: HTMLProps<'bdo', HTMLAttributes>;
        big: HTMLProps<'big', HTMLAttributes>;
        blockquote: HTMLProps<'blockquote', HTMLAttributes>;
        body: HTMLProps<'body', HTMLAttributes>;
        br: HTMLProps<'br', HTMLAttributes>;
        button: HTMLProps<'button', HTMLAttributes>;
        canvas: HTMLProps<'canvas', HTMLAttributes>;
        caption: HTMLProps<'caption', HTMLAttributes>;
        cite: HTMLProps<'cite', HTMLAttributes>;
        code: HTMLProps<'code', HTMLAttributes>;
        col: HTMLProps<'col', HTMLAttributes>;
        colgroup: HTMLProps<'colgroup', HTMLAttributes>;
        data: HTMLProps<'data', HTMLAttributes>;
        datalist: HTMLProps<'datalist', HTMLAttributes>;
        dd: HTMLProps<'dd', HTMLAttributes>;
        del: HTMLProps<'del', HTMLAttributes>;
        details: HTMLProps<'details', HTMLAttributes>;
        dfn: HTMLProps<'dfn', HTMLAttributes>;
        dialog: HTMLProps<'dialog', HTMLAttributes>;
        div: HTMLProps<'div', HTMLAttributes>;
        dl: HTMLProps<'dl', HTMLAttributes>;
        dt: HTMLProps<'dt', HTMLAttributes>;
        em: HTMLProps<'em', HTMLAttributes>;
        embed: HTMLProps<'embed', HTMLAttributes>;
        fieldset: HTMLProps<'fieldset', HTMLAttributes>;
        figcaption: HTMLProps<'figcaption', HTMLAttributes>;
        figure: HTMLProps<'figure', HTMLAttributes>;
        footer: HTMLProps<'footer', HTMLAttributes>;
        form: HTMLProps<'form', HTMLAttributes>;
        h1: HTMLProps<'h1', HTMLAttributes>;
        h2: HTMLProps<'h2', HTMLAttributes>;
        h3: HTMLProps<'h3', HTMLAttributes>;
        h4: HTMLProps<'h4', HTMLAttributes>;
        h5: HTMLProps<'h5', HTMLAttributes>;
        h6: HTMLProps<'h6', HTMLAttributes>;
        head: HTMLProps<'head', HTMLAttributes>;
        header: HTMLProps<'header', HTMLAttributes>;
        hgroup: HTMLProps<'hgroup', HTMLAttributes>;
        hr: HTMLProps<'hr', HTMLAttributes>;
        html: HTMLProps<'html', HTMLAttributes>;
        i: HTMLProps<'i', HTMLAttributes>;
        iframe: HTMLProps<'iframe', HTMLAttributes>;
        img: HTMLProps<'img', HTMLAttributes>;
        input: HTMLProps<'input', HTMLAttributes>;
        ins: HTMLProps<'ins', HTMLAttributes>;
        kbd: HTMLProps<'kbd', HTMLAttributes>;
        keygen: HTMLProps<'keygen', HTMLAttributes>;
        label: HTMLProps<'label', HTMLAttributes>;
        legend: HTMLProps<'legend', HTMLAttributes>;
        li: HTMLProps<'li', HTMLAttributes>;
        link: HTMLProps<'link', HTMLAttributes>;
        main: HTMLProps<'main', HTMLAttributes>;
        map: HTMLProps<'map', HTMLAttributes>;
        mark: HTMLProps<'mark', HTMLAttributes>;
        menu: HTMLProps<'menu', HTMLAttributes>;
        menuitem: HTMLProps<'menuitem', HTMLAttributes>;
        meta: HTMLProps<'meta', HTMLAttributes>;
        meter: HTMLProps<'meter', HTMLAttributes>;
        nav: HTMLProps<'nav', HTMLAttributes>;
        noscript: HTMLProps<'noscript', HTMLAttributes>;
        object: HTMLProps<'object', HTMLAttributes>;
        ol: HTMLProps<'ol', HTMLAttributes>;
        optgroup: HTMLProps<'optgroup', HTMLAttributes>;
        option: HTMLProps<'option', HTMLAttributes>;
        output: HTMLProps<'output', HTMLAttributes>;
        p: HTMLProps<'p', HTMLAttributes>;
        param: HTMLProps<'param', HTMLAttributes>;
        picture: HTMLProps<'picture', HTMLAttributes>;
        pre: HTMLProps<'pre', HTMLAttributes>;
        progress: HTMLProps<'progress', HTMLAttributes>;
        q: HTMLProps<'q', HTMLAttributes>;
        rp: HTMLProps<'rp', HTMLAttributes>;
        rt: HTMLProps<'rt', HTMLAttributes>;
        ruby: HTMLProps<'ruby', HTMLAttributes>;
        s: HTMLProps<'s', HTMLAttributes>;
        samp: HTMLProps<'samp', HTMLAttributes>;
        slot: HTMLProps<'slot', HTMLAttributes>;
        script: HTMLProps<'script', HTMLAttributes>;
        section: HTMLProps<'section', HTMLAttributes>;
        select: HTMLProps<'select', HTMLAttributes>;
        small: HTMLProps<'small', HTMLAttributes>;
        source: HTMLProps<'source', HTMLAttributes>;
        span: HTMLProps<'span', HTMLAttributes>;
        strong: HTMLProps<'strong', HTMLAttributes>;
        style: HTMLProps<'style', HTMLAttributes>;
        sub: HTMLProps<'sub', HTMLAttributes>;
        summary: HTMLProps<'summary', HTMLAttributes>;
        sup: HTMLProps<'sup', HTMLAttributes>;
        table: HTMLProps<'table', HTMLAttributes>;
        template: HTMLProps<'template', HTMLAttributes>;
        tbody: HTMLProps<'tbody', HTMLAttributes>;
        td: HTMLProps<'td', HTMLAttributes>;
        textarea: HTMLProps<'textarea', HTMLAttributes>;
        tfoot: HTMLProps<'tfoot', HTMLAttributes>;
        th: HTMLProps<'th', HTMLAttributes>;
        thead: HTMLProps<'thead', HTMLAttributes>;
        time: HTMLProps<'time', HTMLAttributes>;
        title: HTMLProps<'title', HTMLAttributes>;
        tr: HTMLProps<'tr', HTMLAttributes>;
        track: HTMLProps<'track', HTMLAttributes>;
        u: HTMLProps<'u', HTMLAttributes>;
        ul: HTMLProps<'ul', HTMLAttributes>;
        var: HTMLProps<'var', HTMLAttributes>;
        video: HTMLProps<'video', HTMLAttributes>;
        wbr: HTMLProps<'wbr', HTMLAttributes>;
        webview: HTMLProps<'webview', HTMLAttributes>;
        // SVG
        svg: HTMLProps<'svg', SVGAttributes>;

        animate: HTMLProps<'animate', SVGAttributes>;
        animateMotion: HTMLProps<'animateMotion', SVGAttributes>;
        animateTransform: HTMLProps<'animateTransform', SVGAttributes>;
        circle: HTMLProps<'circle', SVGAttributes>;
        clipPath: HTMLProps<'clipPath', SVGAttributes>;
        defs: HTMLProps<'defs', SVGAttributes>;
        desc: HTMLProps<'desc', SVGAttributes>;
        ellipse: HTMLProps<'ellipse', SVGAttributes>;
        feBlend: HTMLProps<'feBlend', SVGAttributes>;
        feColorMatrix: HTMLProps<'feColorMatrix', SVGAttributes>;
        feComponentTransfer: HTMLProps<'feComponentTransfer', SVGAttributes>;
        feComposite: HTMLProps<'feComposite', SVGAttributes>;
        feConvolveMatrix: HTMLProps<'feConvolveMatrix', SVGAttributes>;
        feDiffuseLighting: HTMLProps<'feDiffuseLighting', SVGAttributes>;
        feDisplacementMap: HTMLProps<'feDisplacementMap', SVGAttributes>;
        feDistantLight: HTMLProps<'feDistantLight', SVGAttributes>;
        feDropShadow: HTMLProps<'feDropShadow', SVGAttributes>;
        feFlood: HTMLProps<'feFlood', SVGAttributes>;
        feFuncA: HTMLProps<'feFuncA', SVGAttributes>;
        feFuncB: HTMLProps<'feFuncB', SVGAttributes>;
        feFuncG: HTMLProps<'feFuncG', SVGAttributes>;
        feFuncR: HTMLProps<'feFuncR', SVGAttributes>;
        feGaussianBlur: HTMLProps<'feGaussianBlur', SVGAttributes>;
        feImage: HTMLProps<'feImage', SVGAttributes>;
        feMerge: HTMLProps<'feMerge', SVGAttributes>;
        feMergeNode: HTMLProps<'feMergeNode', SVGAttributes>;
        feMorphology: HTMLProps<'feMorphology', SVGAttributes>;
        feOffset: HTMLProps<'feOffset', SVGAttributes>;
        fePointLight: HTMLProps<'fePointLight', SVGAttributes>;
        feSpecularLighting: HTMLProps<'feSpecularLighting', SVGAttributes>;
        feSpotLight: HTMLProps<'feSpotLight', SVGAttributes>;
        feTile: HTMLProps<'feTile', SVGAttributes>;
        feTurbulence: HTMLProps<'feTurbulence', SVGAttributes>;
        filter: HTMLProps<'filter', SVGAttributes>;
        foreignObject: HTMLProps<'foreignObject', SVGAttributes>;
        g: HTMLProps<'g', SVGAttributes>;
        image: HTMLProps<'image', SVGAttributes>;
        line: HTMLProps<'line', SVGAttributes>;
        linearGradient: HTMLProps<'linearGradient', SVGAttributes>;
        marker: HTMLProps<'marker', SVGAttributes>;
        mask: HTMLProps<'mask', SVGAttributes>;
        metadata: HTMLProps<'metadata', SVGAttributes>;
        mpath: HTMLProps<'mpath', SVGAttributes>;
        path: HTMLProps<'path', SVGAttributes>;
        pattern: HTMLProps<'pattern', SVGAttributes>;
        polygon: HTMLProps<'polygon', SVGAttributes>;
        polyline: HTMLProps<'polyline', SVGAttributes>;
        radialGradient: HTMLProps<'radialGradient', SVGAttributes>;
        rect: HTMLProps<'rect', SVGAttributes>;
        stop: HTMLProps<'stop', SVGAttributes>;
        switch: HTMLProps<'switch', SVGAttributes>;
        symbol: HTMLProps<'symbol', SVGAttributes>;
        text: HTMLProps<'text', SVGAttributes>;
        textPath: HTMLProps<'textPath', SVGAttributes>;
        tspan: HTMLProps<'tspan', SVGAttributes>;
        use: HTMLProps<'use', SVGAttributes>;
        view: HTMLProps<'view', SVGAttributes>;

        // Svelte specific
        'svelte:window': HTMLProps<'svelte:window', HTMLAttributes>;
        'svelte:body': HTMLProps<'svelte:body', HTMLAttributes>;
        'svelte:document': HTMLProps<'svelte:document', HTMLAttributes>;
        'svelte:fragment': { slot?: string };
        'svelte:options': { [name: string]: any };
        'svelte:head': { [name: string]: any };
        // `<svelte:boundary onerror={…} />` (Svelte 5.3+). Mirrors
        // svelte/elements' `'svelte:boundary'` shape so the boundary's
        // callback signatures and snippet shapes type-check at use.
        'svelte:boundary': {
            onerror?: (error: unknown, reset: () => void) => void;
            failed?: import('svelte').Snippet<[error: unknown, reset: () => void]>;
            pending?: import('svelte').Snippet;
        };

        [name: string]: { [name: string]: any };
    }
}

/**
 * Extract the NON-optional Props type from any supported component
 * shape (class or callable). Declared for future bind:prop pair
 * emission — the helper recovers the raw Props type so a
 * local-assignment pair can type-check against the unwrapped slot
 * shape even when __svn_ensure_component wraps it in `Partial<>` for
 * call-site ergonomics.
 *
 * Order matters here too: the class branch has to come first so
 * `new (...) => { $$prop_def: P }` binds before the callable branch
 * reinterprets the class's constructor signature as a plain callable.
 */
type __SvnProps<C> =
    C extends new (...args: any[]) => { $$prop_def: infer P } ? P :
    C extends (anchor: any, props: infer P) => any
        ? (P extends Partial<infer Q> ? Q : P)
        : never;

// ---------- asset side-effect imports ----------
//
// Bundlers (Vite, webpack, etc.) let user code do side-effect imports
// of assets. Two flavours we cover here:
//
//   1. File-extension imports:  `import './styles.css'`,
//      `import 'swiper/bundle.min.css'`. Matches `*.css` pattern —
//      the literal file extension is part of the specifier.
//   2. Package-subpath imports: `import 'swiper/css'`,
//      `import 'swiper/css/navigation'`. These are package
//      `exports`-map subpaths whose specifiers don't end in `.css`
//      but resolve to CSS files at runtime. Vite's own package.json
//      exports handles this; tsgo's overlay never sees it and fires
//      TS2307 "Cannot find module 'swiper/css'".
//
// Vite's `vite/client.d.ts` declares the `*.css` ambients but not the
// package-subpath shape. Upstream svelte-check silently accepts
// package subpaths — likely because svelte-kit projects transitively
// load `vite/client` AND the tsgo-side module resolver is more
// permissive on unresolved side-effect imports (no `.ts` extension
// to look for, so bundler auto-extension doesn't fire).
//
// Rather than try to enumerate every package-subpath shape
// (`*/css`, `*/styles.css`, etc.), silence side-effect imports
// generally by accepting the common asset extensions PLUS the
// `swiper/css`-style subpath via `*/css/*` and `*/css` patterns.
// Empty-body ambients resolve content to `{}` — import expressions
// compile to `any` and side-effect imports type-check without
// constraining content.
declare module '*.css' {}
declare module '*.scss' {}
declare module '*.sass' {}
declare module '*.less' {}
declare module '*.styl' {}
declare module '*.stylus' {}
declare module '*.pcss' {}
declare module '*.postcss' {}
// Package-subpath CSS imports (swiper/css, etc.). TS module patterns
// allow at most ONE `*` character per declaration, so the previous
// `'*/css/*'` form fired TS2696 against tsgo when `skipLibCheck` is
// off. Single-wildcard `'*/css'` covers the common `import 'pkg/css'`
// shape; deeper subpaths (`pkg/css/variant`) need real types from the
// publishing package or a project-specific declaration.
declare module '*/css' {}

//
// We declare only what's needed to make type-checking succeed for code
// that imports from the standard `svelte/*` entry points. When the real
// `svelte` package IS installed, its declarations win because they live
// inside node_modules and are loaded first by tsgo's resolver.
//
// This file is regenerated into the cache directory on every check;
// edits here belong in svn-typecheck's source.

// @@FALLBACK_BEGIN@@
// Fallback Svelte module shims — only reach the cache when no real
// `svelte` package is reachable from the workspace's node_modules
// chain. When real svelte IS installed these shims would shadow the
// richer real types and surface false-positive TS2305 errors
// ("module has no exported member named 'X'") on user code that
// imports names we didn't enumerate, so typecheck/src/lib.rs strips
// the whole @@FALLBACK_BEGIN@@ … @@FALLBACK_END@@ block in that case.

declare module 'svelte' {
    export interface ComponentConstructorOptions<
        Props extends Record<string, unknown> = Record<string, unknown>,
    > {
        target: Element | Document | ShadowRoot;
        anchor?: Element;
        props?: Props;
        context?: Map<unknown, unknown>;
        hydrate?: boolean;
        intro?: boolean;
    }

    // Real svelte ships SvelteComponent as a CLASS (not an interface),
    // and our emit's generated default-export extends it. Declaring as
    // a class here so the fallback shim doesn't fire "Cannot extend an
    // interface" on fixtures that don't have the real svelte package
    // installed.
    export class SvelteComponent<
        Props extends Record<string, any> = Record<string, any>,
        Events extends Record<string, any> = Record<string, any>,
        Slots extends Record<string, any> = Record<string, any>,
    > {
        constructor(options: ComponentConstructorOptions<Props>);
        $set(props: Partial<Props>): void;
        $on<K extends Extract<keyof Events, string>>(
            type: K,
            callback: (e: Events[K]) => void,
        ): () => void;
        $destroy(): void;
        // Phantom fields for type inference.
        $$prop_def: Props;
        $$events_def: Events;
        $$slot_def: Slots;
    }

    export class SvelteComponentTyped<
        Props extends Record<string, unknown> = Record<string, unknown>,
        Events extends Record<string, unknown> = Record<string, unknown>,
        Slots extends Record<string, unknown> = Record<string, unknown>,
    > implements SvelteComponent<Props, Events, Slots>
    {
        constructor(options: ComponentConstructorOptions<Props>);
        $set(props: Partial<Props>): void;
        $on<K extends Extract<keyof Events, string>>(
            type: K,
            callback: (e: Events[K]) => void,
        ): () => void;
        $destroy(): void;
        $$prop_def: Props;
        $$events_def: Events;
        $$slot_def: Slots;
    }

    // Svelte 5 `Component` type — function form.
    export type Component<
        Props extends Record<string, any> = Record<string, any>,
        Exports extends Record<string, any> = Record<string, any>,
        Bindings extends keyof Props | '' = string,
    > = (
        ...args: any[]
    ) => {
        props: Props;
        exports: Exports;
        bindings: Bindings;
    };

    export type Snippet<Parameters extends any[] = []> = {
        (...args: Parameters): any;
    };

    // Mirrors svelte's real `ComponentProps<T>` shape closely enough
    // that `satisfies Partial<ComponentProps<typeof X>>` flows the
    // declared prop shape through to arrow-function destructure
    // binding inference. `T = any` (fallback for untyped overlay
    // defaults) degrades to `any`, so satisfies is a no-op there
    // rather than firing false-positive contextual-typing errors.
    export type ComponentProps<T> =
        0 extends 1 & T ? any :
        T extends Component<infer Props, any, any> ? Props :
        T extends SvelteComponent<infer Props, any, any> ? Props :
        any;

    export function onMount(fn: () => void | (() => void)): void;
    export function onDestroy(fn: () => void): void;
    export function beforeUpdate(fn: () => void): void;
    export function afterUpdate(fn: () => void): void;
    export function tick(): Promise<void>;
    export function untrack<T>(fn: () => T): T;
    export function mount<Props extends Record<string, any>>(
        component: Component<Props>,
        options: { target: Element; props?: Props },
    ): { exports: any };
    export function unmount(component: any): Promise<void>;
    export function hydrate<Props extends Record<string, any>>(
        component: Component<Props>,
        options: { target: Element; props?: Props },
    ): { exports: any };
    export function getContext<T = unknown>(key: any): T;
    export function setContext<T>(key: any, value: T): T;
    export function hasContext(key: any): boolean;
    export function getAllContexts<T extends Map<any, any> = Map<any, any>>(): T;
    export function createEventDispatcher<
        Events extends Record<string, unknown> = Record<string, unknown>,
    >(): <K extends Extract<keyof Events, string>>(type: K, detail?: Events[K]) => boolean;
}

declare module 'svelte/store' {
    export interface Subscriber<T> {
        (value: T): void;
    }
    export interface Unsubscriber {
        (): void;
    }
    export interface Updater<T> {
        (value: T): T;
    }
    export interface Readable<T> {
        subscribe(run: Subscriber<T>, invalidate?: () => void): Unsubscriber;
    }
    export interface Writable<T> extends Readable<T> {
        set(this: void, value: T): void;
        update(this: void, updater: Updater<T>): void;
    }

    export type StartStopNotifier<T> = (
        set: (value: T) => void,
        update: (fn: Updater<T>) => void,
    ) => void | (() => void);

    export function readable<T>(value?: T, start?: StartStopNotifier<T>): Readable<T>;
    export function writable<T>(value?: T, start?: StartStopNotifier<T>): Writable<T>;
    export function derived<S extends Readable<unknown> | Array<Readable<unknown>>, T>(
        stores: S,
        fn: (values: any, set?: (value: T) => void) => T | void,
        initial_value?: T,
    ): Readable<T>;
    export function get<T>(store: Readable<T>): T;
    export function readonly<T>(store: Writable<T>): Readable<T>;
}

declare module 'svelte/transition' {
    export interface TransitionConfig {
        delay?: number;
        duration?: number;
        easing?: (t: number) => number;
        css?: (t: number, u: number) => string;
        tick?: (t: number, u: number) => void;
    }
    export type TransitionFn<P = any> = (
        node: Element,
        params?: P,
        options?: { direction?: 'in' | 'out' | 'both' },
    ) => TransitionConfig | (() => TransitionConfig);

    export const fade: TransitionFn<{ delay?: number; duration?: number; easing?: (t: number) => number }>;
    export const blur: TransitionFn<any>;
    export const fly: TransitionFn<any>;
    export const slide: TransitionFn<any>;
    export const scale: TransitionFn<any>;
    // `draw` is path-only — its `getTotalLength()` use limits the
    // node parameter to SVGGeometryElement-shaped DOM. Mirrors real
    // svelte/transition's `draw` signature.
    export const draw: (
        node: SVGElement & { getTotalLength(): number },
        params?: any,
        options?: { direction?: 'in' | 'out' | 'both' },
    ) => TransitionConfig | (() => TransitionConfig);
    export const crossfade: (params?: any) => [TransitionFn<any>, TransitionFn<any>];
}

declare module 'svelte/animate' {
    export interface AnimationConfig {
        delay?: number;
        duration?: number;
        easing?: (t: number) => number;
        css?: (t: number, u: number) => string;
        tick?: (t: number, u: number) => void;
    }
    export const flip: (
        node: Element,
        from: { from: DOMRect; to: DOMRect },
        params?: { delay?: number; duration?: number | ((len: number) => number); easing?: (t: number) => number },
    ) => AnimationConfig;
}

declare module 'svelte/easing' {
    export type EasingFunction = (t: number) => number;
    export const linear: EasingFunction;
    export const backIn: EasingFunction;
    export const backOut: EasingFunction;
    export const backInOut: EasingFunction;
    export const bounceIn: EasingFunction;
    export const bounceOut: EasingFunction;
    export const bounceInOut: EasingFunction;
    export const circIn: EasingFunction;
    export const circOut: EasingFunction;
    export const circInOut: EasingFunction;
    export const cubicIn: EasingFunction;
    export const cubicOut: EasingFunction;
    export const cubicInOut: EasingFunction;
    export const elasticIn: EasingFunction;
    export const elasticOut: EasingFunction;
    export const elasticInOut: EasingFunction;
    export const expoIn: EasingFunction;
    export const expoOut: EasingFunction;
    export const expoInOut: EasingFunction;
    export const quadIn: EasingFunction;
    export const quadOut: EasingFunction;
    export const quadInOut: EasingFunction;
    export const quartIn: EasingFunction;
    export const quartOut: EasingFunction;
    export const quartInOut: EasingFunction;
    export const quintIn: EasingFunction;
    export const quintOut: EasingFunction;
    export const quintInOut: EasingFunction;
    export const sineIn: EasingFunction;
    export const sineOut: EasingFunction;
    export const sineInOut: EasingFunction;
}

declare module 'svelte/motion' {
    import type { Readable } from 'svelte/store';
    export interface Spring<T> extends Readable<T> {
        set(value: T, opts?: { hard?: boolean; soft?: boolean | number }): Promise<void>;
        update(fn: (value: T, target: T) => T, opts?: { hard?: boolean; soft?: boolean | number }): Promise<void>;
        stiffness: number;
        damping: number;
        precision: number;
    }
    export interface Tweened<T> extends Readable<T> {
        set(value: T, opts?: { delay?: number; duration?: number; easing?: (t: number) => number }): Promise<void>;
        update(fn: (value: T, target: T) => T, opts?: any): Promise<void>;
    }
    export function spring<T>(value?: T, opts?: any): Spring<T>;
    export function tweened<T>(value?: T, opts?: any): Tweened<T>;
}

declare module 'svelte/action' {
    export interface ActionReturn<P = any, A = any> {
        update?: (parameter: P) => void;
        destroy?: () => void;
        $$_attributes?: A;
    }
    export interface Action<E extends Element = Element, P = any, A = any> {
        (node: E, parameter?: P): void | ActionReturn<P, A>;
    }
}

declare module 'svelte/legacy' {
    export function createBubbler(): any;
    export function nonpassive<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function passive<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function once<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function self<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function trusted<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function preventDefault<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function stopPropagation<T extends Event>(handler: (event: T) => void): (event: T) => void;
    export function stopImmediatePropagation<T extends Event>(handler: (event: T) => void): (event: T) => void;
}

declare module 'svelte/elements' {
    // Closed `HTMLAttributes` shape: standard HTML attrs plus index
    // signatures for `data-*` / `aria-*` and Svelte directive prefixes
    // (`on:*` / `bind:*` / `class:*` / `style:*` / `transition:*` /
    // `in:*` / `out:*` / `animate:*` / `use:*`). Our overlay emits
    // directives as object-literal keys (e.g. `{ "on:click": fn }`),
    // so the directive prefixes need explicit allowance to avoid 2353
    // on legitimate sites. Unknown attributes (no prefix match, no
    // declared property) fire 2353 — this is what unblocks the
    // upstream LS `element-attributes` fixture.
    //
    // Real Svelte's `svelte/elements.HTMLAttributes` enumerates per-
    // event handler types with strict event signatures
    // (`'on:click'?: MouseEventHandler<…>`). Our fallback uses a
    // wildcard `[name: \`on:${string}\`]: any` so we don't regress
    // workspaces that rely on the directive being permissive without
    // a real svelte install. Trade: `<div on:wat>` won't fire 2353
    // here (precise event-name parity needs a vendored svelte stub
    // with full `HTMLElementEventMap` integration; not in scope of
    // this round).
    export interface HTMLAttributes<T extends EventTarget = HTMLElement> {
        // Global HTML attributes
        accesskey?: any;
        autocapitalize?: any;
        autofocus?: any;
        class?: any;
        contenteditable?: any;
        contextmenu?: any;
        dir?: any;
        draggable?: any;
        enterkeyhint?: any;
        hidden?: any;
        id?: any;
        inert?: any;
        inputmode?: any;
        is?: any;
        itemid?: any;
        itemprop?: any;
        itemref?: any;
        itemscope?: any;
        itemtype?: any;
        lang?: any;
        nonce?: any;
        part?: any;
        popover?: any;
        role?: any;
        slot?: any;
        spellcheck?: any;
        style?: any;
        tabindex?: any;
        title?: any;
        translate?: any;
        // Common form/media element attributes — kept in the base for
        // permissiveness across input/button/form/img/source/etc. Real
        // Svelte ships per-element subtypes (`HTMLInputAttributes`,
        // etc.) that define these on specific elements; the fallback
        // collapses them into the base shape so `<input type=…>` /
        // `<button type=…>` / `<a href=…>` etc. type-check without
        // per-element narrowing.
        accept?: any;
        action?: any;
        allow?: any;
        alt?: any;
        async?: any;
        autocomplete?: any;
        autoplay?: any;
        capture?: any;
        charset?: any;
        checked?: any;
        cite?: any;
        cols?: any;
        colspan?: any;
        content?: any;
        controls?: any;
        coords?: any;
        crossorigin?: any;
        data?: any;
        datetime?: any;
        decoding?: any;
        default?: any;
        defer?: any;
        disabled?: any;
        download?: any;
        encoding?: any;
        enctype?: any;
        for?: any;
        form?: any;
        formaction?: any;
        formenctype?: any;
        formmethod?: any;
        formnovalidate?: any;
        formtarget?: any;
        headers?: any;
        height?: any;
        high?: any;
        href?: any;
        hreflang?: any;
        httpEquiv?: any;
        icon?: any;
        kind?: any;
        label?: any;
        list?: any;
        loading?: any;
        loop?: any;
        low?: any;
        manifest?: any;
        max?: any;
        maxlength?: any;
        media?: any;
        method?: any;
        min?: any;
        minlength?: any;
        multiple?: any;
        muted?: any;
        name?: any;
        novalidate?: any;
        open?: any;
        optimum?: any;
        pattern?: any;
        ping?: any;
        placeholder?: any;
        playsinline?: any;
        poster?: any;
        preload?: any;
        readonly?: any;
        referrerpolicy?: any;
        rel?: any;
        required?: any;
        reversed?: any;
        rows?: any;
        rowspan?: any;
        sandbox?: any;
        scope?: any;
        selected?: any;
        shape?: any;
        size?: any;
        sizes?: any;
        span?: any;
        src?: any;
        srcdoc?: any;
        srclang?: any;
        srcset?: any;
        start?: any;
        step?: any;
        summary?: any;
        target?: any;
        type?: any;
        usemap?: any;
        value?: any;
        width?: any;
        wrap?: any;
        // Standard prefixed attribute index signatures
        [name: `data-${string}`]: any;
        [name: `aria-${string}`]: any;
        // Per-event entries — mirrors real svelte's
        // `svelte/elements.d.ts:91-465` byte-for-byte. Both forms
        // ship: `'on:NAME'?` (Svelte 4 directive) and `onNAME?` /
        // `onNAMEcapture?` (Svelte 5 native attr). Replacing the
        // earlier `[name: `on${string}`]: any` wildcard fires 2353
        // on unknown event names and 2339 on bad event-shape uses
        // (e.g. `on:click={e => e.asd}` — `e` narrows to MouseEvent).
        // Without this, both diagnostics fall through to the
        // permissive wildcard. The other directive prefixes
        // (`bind:`, `class:`, `style:`, `transition:`, `in:`, `out:`,
        // `animate:`, `use:`) remain wildcard since the overlay
        // generates one key per directive site and we don't pretend
        // to validate action signatures here.
        // Clipboard Events
        'on:copy'?: ClipboardEventHandler<T> | undefined | null;
        oncopy?: ClipboardEventHandler<T> | undefined | null;
        oncopycapture?: ClipboardEventHandler<T> | undefined | null;
        'on:cut'?: ClipboardEventHandler<T> | undefined | null;
        oncut?: ClipboardEventHandler<T> | undefined | null;
        oncutcapture?: ClipboardEventHandler<T> | undefined | null;
        'on:paste'?: ClipboardEventHandler<T> | undefined | null;
        onpaste?: ClipboardEventHandler<T> | undefined | null;
        onpastecapture?: ClipboardEventHandler<T> | undefined | null;
        // Composition Events
        'on:compositionend'?: CompositionEventHandler<T> | undefined | null;
        oncompositionend?: CompositionEventHandler<T> | undefined | null;
        oncompositionendcapture?: CompositionEventHandler<T> | undefined | null;
        'on:compositionstart'?: CompositionEventHandler<T> | undefined | null;
        oncompositionstart?: CompositionEventHandler<T> | undefined | null;
        oncompositionstartcapture?: CompositionEventHandler<T> | undefined | null;
        'on:compositionupdate'?: CompositionEventHandler<T> | undefined | null;
        oncompositionupdate?: CompositionEventHandler<T> | undefined | null;
        oncompositionupdatecapture?: CompositionEventHandler<T> | undefined | null;
        // Focus Events
        'on:focus'?: FocusEventHandler<T> | undefined | null;
        onfocus?: FocusEventHandler<T> | undefined | null;
        onfocuscapture?: FocusEventHandler<T> | undefined | null;
        'on:focusin'?: FocusEventHandler<T> | undefined | null;
        onfocusin?: FocusEventHandler<T> | undefined | null;
        onfocusincapture?: FocusEventHandler<T> | undefined | null;
        'on:focusout'?: FocusEventHandler<T> | undefined | null;
        onfocusout?: FocusEventHandler<T> | undefined | null;
        onfocusoutcapture?: FocusEventHandler<T> | undefined | null;
        'on:blur'?: FocusEventHandler<T> | undefined | null;
        onblur?: FocusEventHandler<T> | undefined | null;
        onblurcapture?: FocusEventHandler<T> | undefined | null;
        // Form Events
        'on:change'?: FormEventHandler<T> | undefined | null;
        onchange?: FormEventHandler<T> | undefined | null;
        onchangecapture?: FormEventHandler<T> | undefined | null;
        'on:beforeinput'?: EventHandler<InputEvent, T> | undefined | null;
        onbeforeinput?: EventHandler<InputEvent, T> | undefined | null;
        onbeforeinputcapture?: EventHandler<InputEvent, T> | undefined | null;
        'on:input'?: FormEventHandler<T> | undefined | null;
        oninput?: FormEventHandler<T> | undefined | null;
        oninputcapture?: FormEventHandler<T> | undefined | null;
        'on:reset'?: FormEventHandler<T> | undefined | null;
        onreset?: FormEventHandler<T> | undefined | null;
        onresetcapture?: FormEventHandler<T> | undefined | null;
        'on:submit'?: EventHandler<SubmitEvent, T> | undefined | null;
        onsubmit?: EventHandler<SubmitEvent, T> | undefined | null;
        onsubmitcapture?: EventHandler<SubmitEvent, T> | undefined | null;
        'on:invalid'?: EventHandler<Event, T> | undefined | null;
        oninvalid?: EventHandler<Event, T> | undefined | null;
        oninvalidcapture?: EventHandler<Event, T> | undefined | null;
        'on:formdata'?: EventHandler<FormDataEvent, T> | undefined | null;
        onformdata?: EventHandler<FormDataEvent, T> | undefined | null;
        onformdatacapture?: EventHandler<FormDataEvent, T> | undefined | null;
        // Image Events
        'on:load'?: EventHandler | undefined | null;
        onload?: EventHandler | undefined | null;
        onloadcapture?: EventHandler | undefined | null;
        'on:error'?: EventHandler | undefined | null;
        onerror?: EventHandler | undefined | null;
        onerrorcapture?: EventHandler | undefined | null;
        // Popover Events
        'on:beforetoggle'?: ToggleEventHandler<T> | undefined | null;
        onbeforetoggle?: ToggleEventHandler<T> | undefined | null;
        onbeforetogglecapture?: ToggleEventHandler<T> | undefined | null;
        'on:toggle'?: ToggleEventHandler<T> | undefined | null;
        ontoggle?: ToggleEventHandler<T> | undefined | null;
        ontogglecapture?: ToggleEventHandler<T> | undefined | null;
        // Content visibility Events
        'on:contentvisibilityautostatechange'?: ContentVisibilityAutoStateChangeEventHandler<T> | undefined | null;
        oncontentvisibilityautostatechange?: ContentVisibilityAutoStateChangeEventHandler<T> | undefined | null;
        oncontentvisibilityautostatechangecapture?: ContentVisibilityAutoStateChangeEventHandler<T> | undefined | null;
        // Keyboard Events
        'on:keydown'?: KeyboardEventHandler<T> | undefined | null;
        onkeydown?: KeyboardEventHandler<T> | undefined | null;
        onkeydowncapture?: KeyboardEventHandler<T> | undefined | null;
        'on:keypress'?: KeyboardEventHandler<T> | undefined | null;
        onkeypress?: KeyboardEventHandler<T> | undefined | null;
        onkeypresscapture?: KeyboardEventHandler<T> | undefined | null;
        'on:keyup'?: KeyboardEventHandler<T> | undefined | null;
        onkeyup?: KeyboardEventHandler<T> | undefined | null;
        onkeyupcapture?: KeyboardEventHandler<T> | undefined | null;
        // Media Events
        'on:abort'?: EventHandler<Event, T> | undefined | null;
        onabort?: EventHandler<Event, T> | undefined | null;
        onabortcapture?: EventHandler<Event, T> | undefined | null;
        'on:canplay'?: EventHandler<Event, T> | undefined | null;
        oncanplay?: EventHandler<Event, T> | undefined | null;
        oncanplaycapture?: EventHandler<Event, T> | undefined | null;
        'on:canplaythrough'?: EventHandler<Event, T> | undefined | null;
        oncanplaythrough?: EventHandler<Event, T> | undefined | null;
        oncanplaythroughcapture?: EventHandler<Event, T> | undefined | null;
        'on:cuechange'?: EventHandler<Event, T> | undefined | null;
        oncuechange?: EventHandler<Event, T> | undefined | null;
        oncuechangecapture?: EventHandler<Event, T> | undefined | null;
        'on:durationchange'?: EventHandler<Event, T> | undefined | null;
        ondurationchange?: EventHandler<Event, T> | undefined | null;
        ondurationchangecapture?: EventHandler<Event, T> | undefined | null;
        'on:emptied'?: EventHandler<Event, T> | undefined | null;
        onemptied?: EventHandler<Event, T> | undefined | null;
        onemptiedcapture?: EventHandler<Event, T> | undefined | null;
        'on:encrypted'?: EventHandler<Event, T> | undefined | null;
        onencrypted?: EventHandler<Event, T> | undefined | null;
        onencryptedcapture?: EventHandler<Event, T> | undefined | null;
        'on:ended'?: EventHandler<Event, T> | undefined | null;
        onended?: EventHandler<Event, T> | undefined | null;
        onendedcapture?: EventHandler<Event, T> | undefined | null;
        'on:loadeddata'?: EventHandler<Event, T> | undefined | null;
        onloadeddata?: EventHandler<Event, T> | undefined | null;
        onloadeddatacapture?: EventHandler<Event, T> | undefined | null;
        'on:loadedmetadata'?: EventHandler<Event, T> | undefined | null;
        onloadedmetadata?: EventHandler<Event, T> | undefined | null;
        onloadedmetadatacapture?: EventHandler<Event, T> | undefined | null;
        'on:loadstart'?: EventHandler<Event, T> | undefined | null;
        onloadstart?: EventHandler<Event, T> | undefined | null;
        onloadstartcapture?: EventHandler<Event, T> | undefined | null;
        'on:pause'?: EventHandler<Event, T> | undefined | null;
        onpause?: EventHandler<Event, T> | undefined | null;
        onpausecapture?: EventHandler<Event, T> | undefined | null;
        'on:play'?: EventHandler<Event, T> | undefined | null;
        onplay?: EventHandler<Event, T> | undefined | null;
        onplaycapture?: EventHandler<Event, T> | undefined | null;
        'on:playing'?: EventHandler<Event, T> | undefined | null;
        onplaying?: EventHandler<Event, T> | undefined | null;
        onplayingcapture?: EventHandler<Event, T> | undefined | null;
        'on:progress'?: EventHandler<Event, T> | undefined | null;
        onprogress?: EventHandler<Event, T> | undefined | null;
        onprogresscapture?: EventHandler<Event, T> | undefined | null;
        'on:ratechange'?: EventHandler<Event, T> | undefined | null;
        onratechange?: EventHandler<Event, T> | undefined | null;
        onratechangecapture?: EventHandler<Event, T> | undefined | null;
        'on:seeked'?: EventHandler<Event, T> | undefined | null;
        onseeked?: EventHandler<Event, T> | undefined | null;
        onseekedcapture?: EventHandler<Event, T> | undefined | null;
        'on:seeking'?: EventHandler<Event, T> | undefined | null;
        onseeking?: EventHandler<Event, T> | undefined | null;
        onseekingcapture?: EventHandler<Event, T> | undefined | null;
        'on:stalled'?: EventHandler<Event, T> | undefined | null;
        onstalled?: EventHandler<Event, T> | undefined | null;
        onstalledcapture?: EventHandler<Event, T> | undefined | null;
        'on:suspend'?: EventHandler<Event, T> | undefined | null;
        onsuspend?: EventHandler<Event, T> | undefined | null;
        onsuspendcapture?: EventHandler<Event, T> | undefined | null;
        'on:timeupdate'?: EventHandler<Event, T> | undefined | null;
        ontimeupdate?: EventHandler<Event, T> | undefined | null;
        ontimeupdatecapture?: EventHandler<Event, T> | undefined | null;
        'on:volumechange'?: EventHandler<Event, T> | undefined | null;
        onvolumechange?: EventHandler<Event, T> | undefined | null;
        onvolumechangecapture?: EventHandler<Event, T> | undefined | null;
        'on:waiting'?: EventHandler<Event, T> | undefined | null;
        onwaiting?: EventHandler<Event, T> | undefined | null;
        onwaitingcapture?: EventHandler<Event, T> | undefined | null;
        // MouseEvents
        'on:auxclick'?: MouseEventHandler<T> | undefined | null;
        onauxclick?: MouseEventHandler<T> | undefined | null;
        onauxclickcapture?: MouseEventHandler<T> | undefined | null;
        'on:click'?: MouseEventHandler<T> | undefined | null;
        onclick?: MouseEventHandler<T> | undefined | null;
        onclickcapture?: MouseEventHandler<T> | undefined | null;
        'on:contextmenu'?: MouseEventHandler<T> | undefined | null;
        oncontextmenu?: MouseEventHandler<T> | undefined | null;
        oncontextmenucapture?: MouseEventHandler<T> | undefined | null;
        'on:dblclick'?: MouseEventHandler<T> | undefined | null;
        ondblclick?: MouseEventHandler<T> | undefined | null;
        ondblclickcapture?: MouseEventHandler<T> | undefined | null;
        'on:drag'?: DragEventHandler<T> | undefined | null;
        ondrag?: DragEventHandler<T> | undefined | null;
        ondragcapture?: DragEventHandler<T> | undefined | null;
        'on:dragend'?: DragEventHandler<T> | undefined | null;
        ondragend?: DragEventHandler<T> | undefined | null;
        ondragendcapture?: DragEventHandler<T> | undefined | null;
        'on:dragenter'?: DragEventHandler<T> | undefined | null;
        ondragenter?: DragEventHandler<T> | undefined | null;
        ondragentercapture?: DragEventHandler<T> | undefined | null;
        'on:dragexit'?: DragEventHandler<T> | undefined | null;
        ondragexit?: DragEventHandler<T> | undefined | null;
        ondragexitcapture?: DragEventHandler<T> | undefined | null;
        'on:dragleave'?: DragEventHandler<T> | undefined | null;
        ondragleave?: DragEventHandler<T> | undefined | null;
        ondragleavecapture?: DragEventHandler<T> | undefined | null;
        'on:dragover'?: DragEventHandler<T> | undefined | null;
        ondragover?: DragEventHandler<T> | undefined | null;
        ondragovercapture?: DragEventHandler<T> | undefined | null;
        'on:dragstart'?: DragEventHandler<T> | undefined | null;
        ondragstart?: DragEventHandler<T> | undefined | null;
        ondragstartcapture?: DragEventHandler<T> | undefined | null;
        'on:drop'?: DragEventHandler<T> | undefined | null;
        ondrop?: DragEventHandler<T> | undefined | null;
        ondropcapture?: DragEventHandler<T> | undefined | null;
        'on:mousedown'?: MouseEventHandler<T> | undefined | null;
        onmousedown?: MouseEventHandler<T> | undefined | null;
        onmousedowncapture?: MouseEventHandler<T> | undefined | null;
        'on:mouseenter'?: MouseEventHandler<T> | undefined | null;
        onmouseenter?: MouseEventHandler<T> | undefined | null;
        'on:mouseleave'?: MouseEventHandler<T> | undefined | null;
        onmouseleave?: MouseEventHandler<T> | undefined | null;
        'on:mousemove'?: MouseEventHandler<T> | undefined | null;
        onmousemove?: MouseEventHandler<T> | undefined | null;
        onmousemovecapture?: MouseEventHandler<T> | undefined | null;
        'on:mouseout'?: MouseEventHandler<T> | undefined | null;
        onmouseout?: MouseEventHandler<T> | undefined | null;
        onmouseoutcapture?: MouseEventHandler<T> | undefined | null;
        'on:mouseover'?: MouseEventHandler<T> | undefined | null;
        onmouseover?: MouseEventHandler<T> | undefined | null;
        onmouseovercapture?: MouseEventHandler<T> | undefined | null;
        'on:mouseup'?: MouseEventHandler<T> | undefined | null;
        onmouseup?: MouseEventHandler<T> | undefined | null;
        onmouseupcapture?: MouseEventHandler<T> | undefined | null;
        // Selection Events
        'on:select'?: EventHandler<Event, T> | undefined | null;
        onselect?: EventHandler<Event, T> | undefined | null;
        onselectcapture?: EventHandler<Event, T> | undefined | null;
        'on:selectionchange'?: EventHandler<Event, T> | undefined | null;
        onselectionchange?: EventHandler<Event, T> | undefined | null;
        onselectionchangecapture?: EventHandler<Event, T> | undefined | null;
        'on:selectstart'?: EventHandler<Event, T> | undefined | null;
        onselectstart?: EventHandler<Event, T> | undefined | null;
        onselectstartcapture?: EventHandler<Event, T> | undefined | null;
        // Touch Events
        'on:touchcancel'?: TouchEventHandler<T> | undefined | null;
        ontouchcancel?: TouchEventHandler<T> | undefined | null;
        ontouchcancelcapture?: TouchEventHandler<T> | undefined | null;
        'on:touchend'?: TouchEventHandler<T> | undefined | null;
        ontouchend?: TouchEventHandler<T> | undefined | null;
        ontouchendcapture?: TouchEventHandler<T> | undefined | null;
        'on:touchmove'?: TouchEventHandler<T> | undefined | null;
        ontouchmove?: TouchEventHandler<T> | undefined | null;
        ontouchmovecapture?: TouchEventHandler<T> | undefined | null;
        'on:touchstart'?: TouchEventHandler<T> | undefined | null;
        ontouchstart?: TouchEventHandler<T> | undefined | null;
        ontouchstartcapture?: TouchEventHandler<T> | undefined | null;
        // Pointer Events
        'on:gotpointercapture'?: PointerEventHandler<T> | undefined | null;
        ongotpointercapture?: PointerEventHandler<T> | undefined | null;
        ongotpointercapturecapture?: PointerEventHandler<T> | undefined | null;
        'on:pointercancel'?: PointerEventHandler<T> | undefined | null;
        onpointercancel?: PointerEventHandler<T> | undefined | null;
        onpointercancelcapture?: PointerEventHandler<T> | undefined | null;
        'on:pointerdown'?: PointerEventHandler<T> | undefined | null;
        onpointerdown?: PointerEventHandler<T> | undefined | null;
        onpointerdowncapture?: PointerEventHandler<T> | undefined | null;
        'on:pointerenter'?: PointerEventHandler<T> | undefined | null;
        onpointerenter?: PointerEventHandler<T> | undefined | null;
        onpointerentercapture?: PointerEventHandler<T> | undefined | null;
        'on:pointerleave'?: PointerEventHandler<T> | undefined | null;
        onpointerleave?: PointerEventHandler<T> | undefined | null;
        onpointerleavecapture?: PointerEventHandler<T> | undefined | null;
        'on:pointermove'?: PointerEventHandler<T> | undefined | null;
        onpointermove?: PointerEventHandler<T> | undefined | null;
        onpointermovecapture?: PointerEventHandler<T> | undefined | null;
        'on:pointerout'?: PointerEventHandler<T> | undefined | null;
        onpointerout?: PointerEventHandler<T> | undefined | null;
        onpointeroutcapture?: PointerEventHandler<T> | undefined | null;
        'on:pointerover'?: PointerEventHandler<T> | undefined | null;
        onpointerover?: PointerEventHandler<T> | undefined | null;
        onpointerovercapture?: PointerEventHandler<T> | undefined | null;
        'on:pointerup'?: PointerEventHandler<T> | undefined | null;
        onpointerup?: PointerEventHandler<T> | undefined | null;
        onpointerupcapture?: PointerEventHandler<T> | undefined | null;
        'on:lostpointercapture'?: PointerEventHandler<T> | undefined | null;
        onlostpointercapture?: PointerEventHandler<T> | undefined | null;
        onlostpointercapturecapture?: PointerEventHandler<T> | undefined | null;
        // Gamepad Events
        'on:gamepadconnected'?: GamepadEventHandler<T> | undefined | null;
        ongamepadconnected?: GamepadEventHandler<T> | undefined | null;
        'on:gamepaddisconnected'?: GamepadEventHandler<T> | undefined | null;
        ongamepaddisconnected?: GamepadEventHandler<T> | undefined | null;
        // UI Events
        'on:scroll'?: UIEventHandler<T> | undefined | null;
        onscroll?: UIEventHandler<T> | undefined | null;
        onscrollcapture?: UIEventHandler<T> | undefined | null;
        'on:scrollend'?: UIEventHandler<T> | undefined | null;
        onscrollend?: UIEventHandler<T> | undefined | null;
        onscrollendcapture?: UIEventHandler<T> | undefined | null;
        'on:resize'?: UIEventHandler<T> | undefined | null;
        onresize?: UIEventHandler<T> | undefined | null;
        onresizecapture?: UIEventHandler<T> | undefined | null;
        // Wheel Events
        'on:wheel'?: WheelEventHandler<T> | undefined | null;
        onwheel?: WheelEventHandler<T> | undefined | null;
        onwheelcapture?: WheelEventHandler<T> | undefined | null;
        // Animation Events
        'on:animationstart'?: AnimationEventHandler<T> | undefined | null;
        onanimationstart?: AnimationEventHandler<T> | undefined | null;
        onanimationstartcapture?: AnimationEventHandler<T> | undefined | null;
        'on:animationend'?: AnimationEventHandler<T> | undefined | null;
        onanimationend?: AnimationEventHandler<T> | undefined | null;
        onanimationendcapture?: AnimationEventHandler<T> | undefined | null;
        'on:animationiteration'?: AnimationEventHandler<T> | undefined | null;
        onanimationiteration?: AnimationEventHandler<T> | undefined | null;
        onanimationiterationcapture?: AnimationEventHandler<T> | undefined | null;
        // Transition Events
        'on:transitionstart'?: TransitionEventHandler<T> | undefined | null;
        ontransitionstart?: TransitionEventHandler<T> | undefined | null;
        ontransitionstartcapture?: TransitionEventHandler<T> | undefined | null;
        'on:transitionrun'?: TransitionEventHandler<T> | undefined | null;
        ontransitionrun?: TransitionEventHandler<T> | undefined | null;
        ontransitionruncapture?: TransitionEventHandler<T> | undefined | null;
        'on:transitionend'?: TransitionEventHandler<T> | undefined | null;
        ontransitionend?: TransitionEventHandler<T> | undefined | null;
        ontransitionendcapture?: TransitionEventHandler<T> | undefined | null;
        'on:transitioncancel'?: TransitionEventHandler<T> | undefined | null;
        ontransitioncancel?: TransitionEventHandler<T> | undefined | null;
        ontransitioncancelcapture?: TransitionEventHandler<T> | undefined | null;
        // Svelte Transition Events
        'on:outrostart'?: EventHandler<CustomEvent<null>, T> | undefined | null;
        onoutrostart?: EventHandler<CustomEvent<null>, T> | undefined | null;
        onoutrostartcapture?: EventHandler<CustomEvent<null>, T> | undefined | null;
        'on:outroend'?: EventHandler<CustomEvent<null>, T> | undefined | null;
        onoutroend?: EventHandler<CustomEvent<null>, T> | undefined | null;
        onoutroendcapture?: EventHandler<CustomEvent<null>, T> | undefined | null;
        'on:introstart'?: EventHandler<CustomEvent<null>, T> | undefined | null;
        onintrostart?: EventHandler<CustomEvent<null>, T> | undefined | null;
        onintrostartcapture?: EventHandler<CustomEvent<null>, T> | undefined | null;
        'on:introend'?: EventHandler<CustomEvent<null>, T> | undefined | null;
        onintroend?: EventHandler<CustomEvent<null>, T> | undefined | null;
        onintroendcapture?: EventHandler<CustomEvent<null>, T> | undefined | null;
        // Message Events
        'on:message'?: MessageEventHandler<T> | undefined | null;
        onmessage?: MessageEventHandler<T> | undefined | null;
        onmessagecapture?: MessageEventHandler<T> | undefined | null;
        'on:messageerror'?: MessageEventHandler<T> | undefined | null;
        onmessageerror?: MessageEventHandler<T> | undefined | null;
        onmessageerrorcapture?: MessageEventHandler<T> | undefined | null;
        // Document Events
        'on:visibilitychange'?: EventHandler<Event, T> | undefined | null;
        onvisibilitychange?: EventHandler<Event, T> | undefined | null;
        onvisibilitychangecapture?: EventHandler<Event, T> | undefined | null;
        // Global Events
        'on:beforematch'?: EventHandler<Event, T> | undefined | null;
        onbeforematch?: EventHandler<Event, T> | undefined | null;
        onbeforematchcapture?: EventHandler<Event, T> | undefined | null;
        'on:cancel'?: EventHandler<Event, T> | undefined | null;
        oncancel?: EventHandler<Event, T> | undefined | null;
        oncancelcapture?: EventHandler<Event, T> | undefined | null;
        'on:close'?: EventHandler<Event, T> | undefined | null;
        onclose?: EventHandler<Event, T> | undefined | null;
        onclosecapture?: EventHandler<Event, T> | undefined | null;
        'on:fullscreenchange'?: EventHandler<Event, T> | undefined | null;
        onfullscreenchange?: EventHandler<Event, T> | undefined | null;
        onfullscreenchangecapture?: EventHandler<Event, T> | undefined | null;
        'on:fullscreenerror'?: EventHandler<Event, T> | undefined | null;
        onfullscreenerror?: EventHandler<Event, T> | undefined | null;
        onfullscreenerrorcapture?: EventHandler<Event, T> | undefined | null;
        [name: `bind:${string}`]: any;
        [name: `class:${string}`]: any;
        [name: `style:${string}`]: any;
        [name: `transition:${string}`]: any;
        [name: `in:${string}`]: any;
        [name: `out:${string}`]: any;
        [name: `animate:${string}`]: any;
        [name: `use:${string}`]: any;
    }
    export interface SVGAttributes<T extends EventTarget = SVGElement> extends HTMLAttributes<T> {}
    export type DOMAttributes<T extends EventTarget = Element> = HTMLAttributes<T>;
    // ClassValue mirrors clsx-style accepted shapes — string, array, or
    // object map of class-name → boolean. Real Svelte 5.10+ exports this.
    export type ClassValue = any;
    // Event handler aliases — Svelte 5 re-exports these from
    // svelte/elements as ergonomic shorthands.
    export type EventHandler<E extends Event = Event, T extends EventTarget = Element> =
        (event: E & { currentTarget: EventTarget & T }) => any;
    export type ClipboardEventHandler<T extends EventTarget = Element> = EventHandler<ClipboardEvent, T>;
    export type CompositionEventHandler<T extends EventTarget = Element> = EventHandler<CompositionEvent, T>;
    export type DragEventHandler<T extends EventTarget = Element> = EventHandler<DragEvent, T>;
    export type FocusEventHandler<T extends EventTarget = Element> = EventHandler<FocusEvent, T>;
    export type FormEventHandler<T extends EventTarget = Element> = EventHandler<Event, T>;
    export type ChangeEventHandler<T extends EventTarget = Element> = EventHandler<Event, T>;
    export type KeyboardEventHandler<T extends EventTarget = Element> = EventHandler<KeyboardEvent, T>;
    export type MouseEventHandler<T extends EventTarget = Element> = EventHandler<MouseEvent, T>;
    export type TouchEventHandler<T extends EventTarget = Element> = EventHandler<TouchEvent, T>;
    export type PointerEventHandler<T extends EventTarget = Element> = EventHandler<PointerEvent, T>;
    export type UIEventHandler<T extends EventTarget = Element> = EventHandler<UIEvent, T>;
    export type WheelEventHandler<T extends EventTarget = Element> = EventHandler<WheelEvent, T>;
    export type AnimationEventHandler<T extends EventTarget = Element> = EventHandler<AnimationEvent, T>;
    export type TransitionEventHandler<T extends EventTarget = Element> = EventHandler<TransitionEvent, T>;
    export type GamepadEventHandler<T extends EventTarget = Element> = EventHandler<Event, T>;
    export type MessageEventHandler<T extends EventTarget = Element> = EventHandler<MessageEvent, T>;
    export type ToggleEventHandler<T extends EventTarget = Element> = EventHandler<Event, T>;
    export type ContentVisibilityAutoStateChangeEventHandler<T extends EventTarget = Element> = EventHandler<Event, T>;

    // Element → attribute-shape mapping. The shim's
    // `svelteHTML.HTMLProps<K, Override>` (in the always-shipped
    // portion) does `Omit<SvelteHTMLElements[K], …> & Override`. Real
    // Svelte ships per-element subtypes (`HTMLAnchorAttributes`,
    // `HTMLButtonAttributes`, etc.); the fallback collapses every
    // element to a generic `HTMLAttributes<…>` since the LS
    // diagnostic-fixture suite doesn't exercise per-element narrow
    // overrides. The `Property` type-parameter passed through to
    // diagnostic messages keeps the displayed type readable
    // (`HTMLProps<"div", HTMLAttributes<any>>`).
    export interface SvelteHTMLElements {
        a: HTMLAttributes<HTMLElement>;
        abbr: HTMLAttributes<HTMLElement>;
        address: HTMLAttributes<HTMLElement>;
        area: HTMLAttributes<HTMLElement>;
        article: HTMLAttributes<HTMLElement>;
        aside: HTMLAttributes<HTMLElement>;
        audio: HTMLAttributes<HTMLElement>;
        b: HTMLAttributes<HTMLElement>;
        base: HTMLAttributes<HTMLElement>;
        bdi: HTMLAttributes<HTMLElement>;
        bdo: HTMLAttributes<HTMLElement>;
        big: HTMLAttributes<HTMLElement>;
        blockquote: HTMLAttributes<HTMLElement>;
        body: HTMLAttributes<HTMLElement>;
        br: HTMLAttributes<HTMLElement>;
        button: HTMLAttributes<HTMLElement>;
        canvas: HTMLAttributes<HTMLElement>;
        caption: HTMLAttributes<HTMLElement>;
        cite: HTMLAttributes<HTMLElement>;
        code: HTMLAttributes<HTMLElement>;
        col: HTMLAttributes<HTMLElement>;
        colgroup: HTMLAttributes<HTMLElement>;
        data: HTMLAttributes<HTMLElement>;
        datalist: HTMLAttributes<HTMLElement>;
        dd: HTMLAttributes<HTMLElement>;
        del: HTMLAttributes<HTMLElement>;
        details: HTMLAttributes<HTMLElement>;
        dfn: HTMLAttributes<HTMLElement>;
        dialog: HTMLAttributes<HTMLElement>;
        div: HTMLAttributes<HTMLElement>;
        dl: HTMLAttributes<HTMLElement>;
        dt: HTMLAttributes<HTMLElement>;
        em: HTMLAttributes<HTMLElement>;
        embed: HTMLAttributes<HTMLElement>;
        fieldset: HTMLAttributes<HTMLElement>;
        figcaption: HTMLAttributes<HTMLElement>;
        figure: HTMLAttributes<HTMLElement>;
        footer: HTMLAttributes<HTMLElement>;
        form: HTMLAttributes<HTMLElement>;
        h1: HTMLAttributes<HTMLElement>;
        h2: HTMLAttributes<HTMLElement>;
        h3: HTMLAttributes<HTMLElement>;
        h4: HTMLAttributes<HTMLElement>;
        h5: HTMLAttributes<HTMLElement>;
        h6: HTMLAttributes<HTMLElement>;
        head: HTMLAttributes<HTMLElement>;
        header: HTMLAttributes<HTMLElement>;
        hgroup: HTMLAttributes<HTMLElement>;
        hr: HTMLAttributes<HTMLElement>;
        html: HTMLAttributes<HTMLElement>;
        i: HTMLAttributes<HTMLElement>;
        iframe: HTMLAttributes<HTMLElement>;
        img: HTMLAttributes<HTMLElement>;
        input: HTMLAttributes<HTMLElement>;
        ins: HTMLAttributes<HTMLElement>;
        kbd: HTMLAttributes<HTMLElement>;
        keygen: HTMLAttributes<HTMLElement>;
        label: HTMLAttributes<HTMLElement>;
        legend: HTMLAttributes<HTMLElement>;
        li: HTMLAttributes<HTMLElement>;
        link: HTMLAttributes<HTMLElement>;
        main: HTMLAttributes<HTMLElement>;
        map: HTMLAttributes<HTMLElement>;
        mark: HTMLAttributes<HTMLElement>;
        menu: HTMLAttributes<HTMLElement>;
        menuitem: HTMLAttributes<HTMLElement>;
        meta: HTMLAttributes<HTMLElement>;
        meter: HTMLAttributes<HTMLElement>;
        nav: HTMLAttributes<HTMLElement>;
        noscript: HTMLAttributes<HTMLElement>;
        object: HTMLAttributes<HTMLElement>;
        ol: HTMLAttributes<HTMLElement>;
        optgroup: HTMLAttributes<HTMLElement>;
        option: HTMLAttributes<HTMLElement>;
        output: HTMLAttributes<HTMLElement>;
        p: HTMLAttributes<HTMLElement>;
        param: HTMLAttributes<HTMLElement>;
        picture: HTMLAttributes<HTMLElement>;
        pre: HTMLAttributes<HTMLElement>;
        progress: HTMLAttributes<HTMLElement>;
        q: HTMLAttributes<HTMLElement>;
        rp: HTMLAttributes<HTMLElement>;
        rt: HTMLAttributes<HTMLElement>;
        ruby: HTMLAttributes<HTMLElement>;
        s: HTMLAttributes<HTMLElement>;
        samp: HTMLAttributes<HTMLElement>;
        slot: HTMLAttributes<HTMLElement>;
        script: HTMLAttributes<HTMLElement>;
        search: HTMLAttributes<HTMLElement>;
        section: HTMLAttributes<HTMLElement>;
        select: HTMLAttributes<HTMLElement>;
        small: HTMLAttributes<HTMLElement>;
        source: HTMLAttributes<HTMLElement>;
        span: HTMLAttributes<HTMLElement>;
        strong: HTMLAttributes<HTMLElement>;
        style: HTMLAttributes<HTMLElement>;
        sub: HTMLAttributes<HTMLElement>;
        summary: HTMLAttributes<HTMLElement>;
        sup: HTMLAttributes<HTMLElement>;
        table: HTMLAttributes<HTMLElement>;
        template: HTMLAttributes<HTMLElement>;
        tbody: HTMLAttributes<HTMLElement>;
        td: HTMLAttributes<HTMLElement>;
        textarea: HTMLAttributes<HTMLElement>;
        tfoot: HTMLAttributes<HTMLElement>;
        th: HTMLAttributes<HTMLElement>;
        thead: HTMLAttributes<HTMLElement>;
        time: HTMLAttributes<HTMLElement>;
        title: HTMLAttributes<HTMLElement>;
        tr: HTMLAttributes<HTMLElement>;
        track: HTMLAttributes<HTMLElement>;
        u: HTMLAttributes<HTMLElement>;
        ul: HTMLAttributes<HTMLElement>;
        var: HTMLAttributes<HTMLElement>;
        video: HTMLAttributes<HTMLElement>;
        wbr: HTMLAttributes<HTMLElement>;
        webview: HTMLAttributes<HTMLElement>;
        // SVG
        svg: SVGAttributes<SVGElement>;
        animate: SVGAttributes<SVGElement>;
        animateMotion: SVGAttributes<SVGElement>;
        animateTransform: SVGAttributes<SVGElement>;
        circle: SVGAttributes<SVGElement>;
        clipPath: SVGAttributes<SVGElement>;
        defs: SVGAttributes<SVGElement>;
        desc: SVGAttributes<SVGElement>;
        ellipse: SVGAttributes<SVGElement>;
        feBlend: SVGAttributes<SVGElement>;
        feColorMatrix: SVGAttributes<SVGElement>;
        feComponentTransfer: SVGAttributes<SVGElement>;
        feComposite: SVGAttributes<SVGElement>;
        feConvolveMatrix: SVGAttributes<SVGElement>;
        feDiffuseLighting: SVGAttributes<SVGElement>;
        feDisplacementMap: SVGAttributes<SVGElement>;
        feDistantLight: SVGAttributes<SVGElement>;
        feDropShadow: SVGAttributes<SVGElement>;
        feFlood: SVGAttributes<SVGElement>;
        feFuncA: SVGAttributes<SVGElement>;
        feFuncB: SVGAttributes<SVGElement>;
        feFuncG: SVGAttributes<SVGElement>;
        feFuncR: SVGAttributes<SVGElement>;
        feGaussianBlur: SVGAttributes<SVGElement>;
        feImage: SVGAttributes<SVGElement>;
        feMerge: SVGAttributes<SVGElement>;
        feMergeNode: SVGAttributes<SVGElement>;
        feMorphology: SVGAttributes<SVGElement>;
        feOffset: SVGAttributes<SVGElement>;
        fePointLight: SVGAttributes<SVGElement>;
        feSpecularLighting: SVGAttributes<SVGElement>;
        feSpotLight: SVGAttributes<SVGElement>;
        feTile: SVGAttributes<SVGElement>;
        feTurbulence: SVGAttributes<SVGElement>;
        filter: SVGAttributes<SVGElement>;
        foreignObject: SVGAttributes<SVGElement>;
        g: SVGAttributes<SVGElement>;
        image: SVGAttributes<SVGElement>;
        line: SVGAttributes<SVGElement>;
        linearGradient: SVGAttributes<SVGElement>;
        marker: SVGAttributes<SVGElement>;
        mask: SVGAttributes<SVGElement>;
        metadata: SVGAttributes<SVGElement>;
        mpath: SVGAttributes<SVGElement>;
        path: SVGAttributes<SVGElement>;
        pattern: SVGAttributes<SVGElement>;
        polygon: SVGAttributes<SVGElement>;
        polyline: SVGAttributes<SVGElement>;
        radialGradient: SVGAttributes<SVGElement>;
        rect: SVGAttributes<SVGElement>;
        stop: SVGAttributes<SVGElement>;
        switch: SVGAttributes<SVGElement>;
        symbol: SVGAttributes<SVGElement>;
        text: SVGAttributes<SVGElement>;
        textPath: SVGAttributes<SVGElement>;
        tspan: SVGAttributes<SVGElement>;
        use: SVGAttributes<SVGElement>;
        view: SVGAttributes<SVGElement>;
        // Svelte-specific element keys (also enumerated in svelteHTML.IntrinsicElements).
        'svelte:window': HTMLAttributes<HTMLElement>;
        'svelte:body': HTMLAttributes<HTMLElement>;
        'svelte:document': HTMLAttributes<HTMLElement>;
        'svelte:fragment': HTMLAttributes<HTMLElement>;
        'svelte:options': HTMLAttributes<HTMLElement>;
        'svelte:head': HTMLAttributes<HTMLElement>;
    }
}

declare module 'svelte/compiler' {
    export const VERSION: string;
    export function compile(source: string, options?: any): any;
    export function parse(source: string, options?: any): any;
    export function preprocess(source: string, transformers: any, options?: any): Promise<{ code: string; map: any }>;
    export function walk(ast: any, walker: any): any;
}
// @@FALLBACK_END@@

