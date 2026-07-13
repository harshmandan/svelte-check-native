// Minimal vendored slice of svelte's `Component` type (from
// svelte/types/index.d.ts) so the fixture validates against the real
// generic surface without a node_modules install: the callable's
// return is `{ $on?; $set? } & Exports`, which is exactly what the
// overlay's `ReturnType<typeof __svn_component_default>` instance
// typedef projects at consumers.
declare module 'svelte' {
    export interface Component<
        Props extends Record<string, any> = {},
        Exports extends Record<string, any> = {},
        Bindings extends keyof Props | '' = string
    > {
        (
            this: void,
            internals: unknown,
            props: Props
        ): {
            $on?(type: string, callback: (e: any) => void): () => void;
            $set?(props: Partial<Props>): void;
        } & Exports;
        element?: typeof HTMLElement;
        z_$$bindings?: Bindings;
    }
}
