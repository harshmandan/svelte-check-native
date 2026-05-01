// Companion broken fixture: prove the D-ii bindings post-check fires.

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

declare const Runes: svelte_new.Component<
    { readonly?: string; can_bind?: string },
    { only_bind: () => void },
    'can_bind'
>;

type RunesInstB = ReturnType<typeof Runes> & { $$bindings?: 'can_bind' };
declare const inst: RunesInstB;

// Expect TS2322 — 'readonly' isn't in the bindable union 'can_bind'.
inst.$$bindings = 'readonly';

// Expect TS2322 — 'only_bind' isn't bindable either.
inst.$$bindings = 'only_bind';
export {};
