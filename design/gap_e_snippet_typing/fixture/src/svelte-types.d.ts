// Minimal Svelte type stubs for the Snippet-receiver experiments below.

declare module 'svelte' {
    // Snippet — the unique-return callable shape. The bits-ui pattern is
    // `child?: Snippet<[{ props: Record<string, unknown> }]>` — a snippet
    // that takes ONE argument with a `props` field.
    declare const __snippet_brand: unique symbol;
    export type SnippetReturn = { [__snippet_brand]: 'snippet' };
    export interface Snippet<Args extends unknown[] = []> {
        (this: void, ...args: Args): SnippetReturn;
    }

    export interface ComponentConstructorOptions<Props = {}> {
        target: any;
        props?: Props;
    }

    export class SvelteComponent<P = any, E = any, S = any> {
        $set(props: Partial<P>): void;
        $on(type: string, cb: (e: any) => void): () => void;
        $$prop_def: P;
        $$events_def: E;
        $$slot_def: S;
    }

    export interface Component<
        Props extends Record<string, any> = {},
        Exports extends Record<string, any> = {},
        Bindings extends keyof Props | '' = string,
    > {
        (this: void, internals: unknown, props: Props): {
            $on?: any;
            $set?: any;
        } & Exports;
        z_$$bindings?: Bindings;
    }
}
