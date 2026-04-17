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

/**
 * Emit-only prop-shape extractor used by `satisfies Partial<__SvnComponentProps<typeof X>>`.
 *
 * Matches svelte's built-in `ComponentProps<T>` behaviour when `T` is a
 * `Component<Props>` or `SvelteComponent<Props>` — extracts `Props`.
 * Unlike the built-in, has no `T extends Component | SvelteComponent`
 * constraint at the type-parameter level, so component shapes that don't
 * fit either (namespace re-exports from third-party libs, custom class
 * shapes, etc.) degrade to `any` rather than firing TS2344.
 *
 * `0 extends 1 & T` preserves `any` (matches __SvnEachItem's trick).
 */
type __SvnComponentProps<T> =
    0 extends 1 & T ? any :
    T extends new (...args: any[]) => import('svelte').SvelteComponent<infer P, any, any> ? P :
    T extends new (...args: any[]) => { $$prop_def: infer P } ? P :
    T extends import('svelte').Component<infer P, any, any> ? P :
    T extends import('svelte').SvelteComponent<infer P, any, any> ? P :
    any;

/** Resolved item type for `__svn_each_items`. The `0 extends 1 & T` guard preserves `any` (avoids the conditional-type-distribution-collapses-to-unknown trap). */
type __SvnEachItem<T> = 0 extends 1 & T
    ? any
    : T extends ArrayLike<infer U>
        ? U
        : T extends Iterable<infer U>
            ? U
            : never;

//
// We declare only what's needed to make type-checking succeed for code
// that imports from the standard `svelte/*` entry points. When the real
// `svelte` package IS installed, its declarations win because they live
// inside node_modules and are loaded first by tsgo's resolver.
//
// This file is regenerated into the cache directory on every check;
// edits here belong in svn-typecheck's source.

