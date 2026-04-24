// Subset of the production svn_shims.d.ts — just what the JS-overlay
// fixture exercises. Same .d.ts is universal so it works in JS files too.

declare function __svn_any<T = any>(): T;

type __SvnStore<T> = { subscribe: (run: (value: T) => any, invalidate?: any) => any };
type __SvnStoreValue<S> = S extends __SvnStore<infer T> ? T : S;

// Mirrors production svelte_shims_core.d.ts:264-265 — the simpler
// 2-overload form. The extra `(initial: null)` / `(initial: undefined)`
// overloads tried earlier here resolve T to `unknown`, which survives
// truthy checks and breaks `clearTimeout(t)` even under JS-loose
// inference. Real svelte ships this same 2-overload form.
declare function $state<T>(initial: T): T;
declare function $state<T>(): T | undefined;

declare function $derived<T>(expression: T): T;
declare function $effect(fn: () => void | (() => void)): void;
declare function $props<T = Record<string, any>>(): T;
declare function $bindable<T>(fallback?: T): T;
