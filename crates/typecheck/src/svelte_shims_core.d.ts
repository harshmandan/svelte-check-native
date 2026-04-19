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
// `svelte_shims_fallback.d.ts` holds the `declare module 'svelte/*'`
// blocks that stand in for the real package; that file is written
// ONLY when no real svelte install is reachable from the workspace.
// When real svelte IS installed, those declarations would shadow the
// richer real types (e.g. `HTMLAnchorAttributes` from svelte/elements)
// and produce false-positive TS2305 errors on user code that uses
// names we didn't include in the shim.

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
declare type __SvnSvelte4PropsWiden<P> = ('slot' extends keyof P ? {} : { slot?: string })
    & ('class' extends keyof P ? {} : { class?: string })
    & ('style' extends keyof P ? {} : { style?: string })
    & { [index: string]: any };

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

/** Iterable wrapper for `{#each}` blocks. Accepts arrays, ArrayLike (`{ length: N }`), and any other iterable. */
declare function __svn_each_items<T>(value: T): Iterable<__SvnEachItem<T>>;

/** Resolved item type for `__svn_each_items`. The `0 extends 1 & T` guard preserves `any` (avoids the conditional-type-distribution-collapses-to-unknown trap). */
type __SvnEachItem<T> = 0 extends 1 & T
    ? any
    : T extends ArrayLike<infer U>
        ? U
        : T extends Iterable<infer U>
            ? U
            : never;

/**
 * Fresh `any` placeholder. Used as the anchor / target argument in the
 * emitted `new Comp({ target: __svn_any(), props: {...} })` call.
 *
 * Declared generic with `T = any` so callers can narrow the return at
 * the call site when needed; default usage gets plain `any`.
 */
declare function __svn_any<T = any>(): T;

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
declare function __svn_ensure_component<P extends Record<string, any>, E>(
    c: import('svelte').Component<P> & { readonly __svn_events: E },
): new (options: { target?: any; props?: __SvnPropsPartial<P> }) => __SvnInstanceTyped<P, E>;
declare function __svn_ensure_component<P extends Record<string, any>>(
    c: import('svelte').Component<P>,
): new (options: { target?: any; props?: __SvnPropsPartial<P> }) => __SvnInstance<P>;
declare function __svn_ensure_component<C extends new (...args: any[]) => any>(c: C): C;
declare function __svn_ensure_component<P>(
    c: (anchor: any, props: P) => any,
): new (options: { target?: any; props?: __SvnPropsPartial<P> }) => __SvnInstance<P>;
declare function __svn_ensure_component(
    c: unknown,
): new (options: { target?: any; props?: any }) => __SvnInstance<any>;

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
    $on<K extends keyof E>(
        event: K,
        handler: (e: CustomEvent<E[K]>) => any,
    ): () => void;
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
// Package-subpath CSS (swiper/css, swiper/css/navigation, etc.).
// Conservative: matches any `<pkg>/css` import exactly and any
// `<pkg>/css/<variant>` subpath.
declare module '*/css' {}
declare module '*/css/*' {}

//
// We declare only what's needed to make type-checking succeed for code
// that imports from the standard `svelte/*` entry points. When the real
// `svelte` package IS installed, its declarations win because they live
// inside node_modules and are loaded first by tsgo's resolver.
//
// This file is regenerated into the cache directory on every check;
// edits here belong in svn-typecheck's source.

