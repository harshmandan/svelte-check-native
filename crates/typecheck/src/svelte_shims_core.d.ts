// Core Svelte type shims — always shipped into the project cache.
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

/** `$state<T>(initial?)` declares reactive state. Macro.
 *
 * Four overloads, in order:
 *   1–2. `$state(null)` / `$state(undefined)` — literal-nullish initial.
 *        T is inferred purely from contextual type (the variable's
 *        annotation), NOT from the argument. This is the key fix for
 *        the common bind-this pattern:
 *          `let el: HTMLInputElement | null = $state(null);`
 *        With the naive single overload `<T>(initial: T): T`, TS binds
 *        T to `null`, narrows the initializer's type to `null`, and
 *        CFA then narrows `el` to `null`. Later `if (el) el.focus()`
 *        sees `el: never` — because the declared annotation was
 *        merely a widening hint, not a fresh type. Splitting
 *        null/undefined into their own overloads lets T remain a free
 *        type variable that TS can fill from the assignment context,
 *        so the returned type matches the annotation verbatim and no
 *        narrowing collapse happens.
 *   3. `$state(value)` — normal initial. T inferred from the argument.
 *   4. `$state()` — no initial. Return is `T | undefined`.
 *
 * Calls like `$state<T>(0)` where T is a generic parameter and 0 isn't
 * assignable to T still fire TS2345 — matches Svelte's own behavior.
 */
declare function $state<T>(initial: null): T;
declare function $state<T>(initial: undefined): T;
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
declare function __svn_ensure_component<P extends Record<string, any>>(
    c: import('svelte').Component<P>,
): new (options: { target?: any; props?: __SvnPropsPartial<P> }) => { $$prop_def: P };
declare function __svn_ensure_component<C extends new (...args: any[]) => any>(c: C): C;
declare function __svn_ensure_component<P>(
    c: (anchor: any, props: P) => any,
): new (options: { target?: any; props?: __SvnPropsPartial<P> }) => { $$prop_def: P };
declare function __svn_ensure_component(
    c: unknown,
): new (options: { target?: any; props?: any }) => { $$prop_def: any };

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

/**
 * Assert that a `bind:this` target's declared type accepts the element
 * shape produced at the bind site. Called as:
 *
 *     __svn_bind_this_check<HTMLInputElement>(inputEl);
 *
 * `target: El | null | undefined` requires `inputEl`'s declared type to
 * be a subtype of `El | null | undefined`, matching the runtime
 * contract: Svelte assigns either the element or nothing. Accepts
 * `HTMLInputElement`, `HTMLInputElement | null`, `HTMLInputElement |
 * undefined`, and the full triplet; rejects a wrong element type
 * (`HTMLDivElement` vs `HTMLInputElement`).
 *
 * Replaces the pre-refactor habit of emitting
 * `inputEl = null as any as HTMLElement | null`, which forced `null`
 * onto the user's variable type and mis-fired on `HTMLElement |
 * undefined`-typed targets (control-repo bug #3).
 */
declare function __svn_bind_this_check<El>(target: El | null | undefined): void;

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


//
// We declare only what's needed to make type-checking succeed for code
// that imports from the standard `svelte/*` entry points. When the real
// `svelte` package IS installed, its declarations win because they live
// inside node_modules and are loaded first by tsgo's resolver.
//
// This file is regenerated into the cache directory on every check;
// edits here belong in svn-typecheck's source.

