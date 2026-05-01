// Tsgo validation fixture for the D-ii bindings cluster.
//
// Goal 1: prove the new shim Component<P, X, B> shape (mirroring real
// svelte's interface) returns `Exports & {$set?, $on?}` from its call
// signature. Consumer-side `let inst: ReturnType<typeof Comp>` then
// has access to `Exports`'s methods directly.
//
// Goal 2: prove the literal-string-union Bindings generic + post-
// instance `inst.$$bindings = 'name'` check fires TS2322 when 'name'
// isn't in the union, and stays silent when it is.

// Mirrors the proposed new svelte_shims_core.d.ts Component shape.
declare namespace svelte_new {
    interface ComponentInternals {}
    interface Component<
        Props extends Record<string, any> = {},
        Exports extends Record<string, any> = {},
        Bindings extends keyof Props | '' = string
    > {
        (this: void, internals: ComponentInternals, props: Props): {
            $set?(props: Partial<Props>): void;
            $on?(type: string, callback: (e: any) => void): () => void;
        } & Exports;
        z_$$bindings?: Bindings;
    }
}

// A Svelte-5 runes-mode component with one bindable prop and one method.
declare const Runes: svelte_new.Component<
    { readonly?: string; can_bind?: string },
    { only_bind: () => void },
    'can_bind'
>;

// Consumer-side: `inst.only_bind()` should resolve via `Exports`.
let inst: ReturnType<typeof Runes>;
inst!.only_bind() === undefined; // OK — only_bind is on Exports

// Bindable prop assignment via the post-instance $$bindings shape.
// Mirror: `__svn_inst_N.$$bindings = 'can_bind'` after the new call.
type RunesInst = ReturnType<typeof Runes> & { $$bindings?: 'can_bind' };
declare const ok_inst: RunesInst;
ok_inst.$$bindings = 'can_bind'; // OK — 'can_bind' is the literal union
export {};
