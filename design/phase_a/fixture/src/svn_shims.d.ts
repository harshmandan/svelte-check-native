// Phase-A candidate helper set for the component-as-callable emit shape.
// These declarations are what our emitter will rely on when it writes
// generated `.svelte.ts` overlay files. Intentionally minimal — every
// helper here has to justify its existence.
//
// Naming: all `__svn_*`. OUR namespace, not upstream's.
//
// This file is a .d.ts script (no imports/exports), so its declarations
// are global — same pattern as svelte_shims_core.d.ts.

// ---------- core shape helpers ----------

/**
 * Produce a fresh `any` value. Used wherever we need to synthesize a
 * placeholder expression whose type must not drive inference — e.g. the
 * first argument to a component-call shape, or a snippet arrow's return
 * value (which has to satisfy `Snippet`'s branded return type).
 *
 * Declared as a generic returning `T` with no constraint so callers can
 * annotate the call site explicitly (`__svn_any<HTMLInputElement>()`)
 * when a specific shape is wanted.
 */
declare function __svn_any<T = any>(): T;

/**
 * Normalize any component shape to a constructible. After wrapping,
 * `new $$_Comp({ target, props })` works uniformly regardless of
 * whether the source component was a class (Svelte-4 style) or a
 * callable (Svelte-5 function / `Component<Props>` type).
 *
 * Two overloads:
 *   1. Class (including our overlay defaults): passthrough — preserves
 *      generics so `new Class<T>({...})` infers T from props.
 *   2. Callable (third-party `Component<Props>` values, including
 *      those typed inline in user-declared contexts): synthesize a
 *      construct signature that carries the same Props.
 *
 * Consumer emits each instantiation as:
 *     { const $$_C0 = __svn_ensure_component(Comp);
 *       new $$_C0({ target: __svn_any(), props: { ... } }); }
 *
 * The extra `const` local is what makes generic inference work: TS
 * binds the helper's return-type generics at the `new` site rather
 * than at the `ensure_component` site.
 */
declare function __svn_ensure_component<C extends new (...args: any[]) => any>(c: C): C;
declare function __svn_ensure_component<P>(c: (anchor: any, props: P) => any):
    new (options: { target?: any; props?: P }) => { $$prop_def: P };
declare function __svn_ensure_component<P>(c: import('svelte').Component<P>):
    new (options: { target?: any; props?: P }) => { $$prop_def: P };
declare function __svn_ensure_component(c: unknown):
    new (options: { target?: any; props?: any }) => { $$prop_def: any };

// ---------- template iteration ----------

/** Iterable wrapper for `{#each}` blocks. */
declare function __svn_each_items<T>(value: T): Iterable<__SvnEachItem<T>>;

type __SvnEachItem<T> = 0 extends 1 & T
    ? any
    : T extends ArrayLike<infer U>
        ? U
        : T extends Iterable<infer U>
            ? U
            : never;

// ---------- store auto-subscribe ----------

type __SvnStore<T> = { subscribe: (run: (value: T) => any, invalidate?: any) => any };
type __SvnStoreValue<S> = S extends __SvnStore<infer T> ? T : S;

// ---------- bind:this ----------

/**
 * Assert that a `bind:this` target's declared type accepts the element
 * shape produced by the tag (or component). Call-site form:
 *
 *     __svn_bind_this_check<HTMLInputElement>(inputEl);
 *
 * Signature requires `inputEl`'s declared type to be a subtype of
 * `HTMLInputElement | null | undefined`, which is the correct contract:
 * at runtime Svelte assigns either the element or nothing. Variables
 * typed as `HTMLInputElement`, `HTMLInputElement | null`,
 * `HTMLInputElement | undefined`, or the full triplet all pass; a
 * wrong-element-type declaration (e.g. `HTMLDivElement`) fails with a
 * clear TS2345 at the call site.
 */
declare function __svn_bind_this_check<El>(target: El | null | undefined): void;

// ---------- snippet return brand ----------

/**
 * Opaque return value for snippet arrow bodies. Svelte's `Snippet<[...]>`
 * type brands its return shape such that a bare `(args) => void` can't
 * structurally satisfy it. Returning `__svn_snippet_return()` produces a
 * value typed as the branded return, so the arrow assigns cleanly
 * through a `Snippet<[...]>` prop slot while contextual typing still
 * flows into the parameters from the caller's signature.
 */
declare function __svn_snippet_return(): any;

// ---------- props extractor for bind:prop ----------

/**
 * Extract the `props` parameter type from a component-as-callable. Used
 * by the bind:prop pair emission to declare a local with the exact prop
 * slot's type, which lets TS check assignability in both directions:
 *
 *     Foo(__svn_any(), { value: userVar });      // user → prop
 *     let __svn_bind_0: __SvnProps<typeof Foo>['value'];
 *     userVar = __svn_bind_0;                    // prop → user
 *
 * For generic components, `Parameters<typeof Foo>` already handles the
 * generic parameter correctly — TS resolves it per call site.
 */
type __SvnProps<C> =
    C extends new (...args: any[]) => { $$prop_def: infer P } ? P :
    C extends (anchor: any, props: infer P) => any
        ? (P extends Partial<infer Q> ? Q : P)
        : C extends import('svelte').Component<infer P> ? P :
    Record<string, any>;

// ---------- rune ambients (mirror svelte_shims_core.d.ts, slimmed for Phase A) ----------

declare function $state<T>(initial: null): T;
declare function $state<T>(initial: undefined): T;
declare function $state<T>(initial: T): T;
declare function $state<T>(): T | undefined;

declare function $derived<T>(expression: T): T;

declare function $effect(fn: () => void | (() => void)): void;

declare function $props<T = Record<string, any>>(): T;

declare function $bindable<T>(fallback?: T): T;
