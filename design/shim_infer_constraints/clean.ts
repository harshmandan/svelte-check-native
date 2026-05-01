// Validates Pre-Tier-1 shim constraints work with `infer X extends Record<string, any>`.
// Mirrors the structure of __SvnComponentEvents/__SvnComponentSlots in
// crates/typecheck/src/svelte_shims_core.d.ts (post-fix).

declare class SvelteComponent<P = any, E extends Record<string, any> = any, S extends Record<string, any> = any> {
    $set(props: Partial<P>): void;
    $on<K extends Extract<keyof E, string>>(type: K, callback: (e: E[K]) => void): () => void;
    $$prop_def: P;
    $$events_def: E;
    $$slot_def: S;
}

type __SvnComponentEvents<C> = C extends { readonly __svn_events: infer E }
    ? E
    : C extends new (...args: any[]) => SvelteComponent<any, infer E extends Record<string, any>, any>
        ? E
        : Record<string, any>;

type __SvnComponentSlots<C> = C extends { readonly __svn_slots: infer S }
    ? S
    : C extends new (...args: any[]) => SvelteComponent<any, any, infer S extends Record<string, any>>
        ? S
        : Record<string, Record<string, any>>;

declare class StrictEvents extends SvelteComponent<{}, { foo: CustomEvent<string>; click: CustomEvent<MouseEvent> }, {}> {}

type E = __SvnComponentEvents<typeof StrictEvents>;
type S = __SvnComponentSlots<typeof StrictEvents>;

// E should resolve to { foo: ...; click: ... }
const e: E = {} as any;
e.foo;   // OK
e.click; // OK
const s: S = {} as any;
void s;
