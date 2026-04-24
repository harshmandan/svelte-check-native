// Rune + helper shims — mirrors the production svelte_shims_core.d.ts subset
// the DOM-element-emit patterns exercise. Kept minimal on purpose; only the
// shapes each pattern file actually uses.

declare function __svn_any<T = any>(): T;

declare function $state<T>(initial: T): T;
declare function $state<T>(): T | undefined;
declare function $derived<T>(expression: T): T;
declare function $props<T = Record<string, any>>(): T;
