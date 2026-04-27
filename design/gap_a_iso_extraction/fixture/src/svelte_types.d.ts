// Minimal Svelte type stubs — just enough surface area for the iso
// shape to typecheck. Real Svelte types live in svelte/types/index.d.ts;
// these mirror them well enough for the assignability tests below.

declare module 'svelte' {
    export interface ComponentConstructorOptions<
        Props extends Record<string, any> = Record<string, any>,
    > {
        target: any;
        props?: Props;
    }

    export class SvelteComponent<
        Props extends Record<string, any> = Record<string, any>,
        Events extends Record<string, any> = any,
        Slots extends Record<string, any> = any,
    > {
        constructor(options: ComponentConstructorOptions<Props>);
        $set(props: Partial<Props>): void;
        $on<K extends Extract<keyof Events, string>>(
            type: K,
            callback: (event: Events[K]) => void,
        ): () => void;
        $destroy(): void;
        $$prop_def: Props;
    }

    export interface Component<
        Props extends Record<string, any> = {},
        Exports extends Record<string, any> = {},
        Bindings extends keyof Props | '' = string,
    > {
        (
            this: void,
            internals: unknown,
            props: Props,
        ): {
            $on?(type: string, callback: (e: any) => void): () => void;
            $set?(props: Partial<Props>): void;
        } & Exports;
        element?: typeof HTMLElement;
        z_$$bindings?: Bindings;
    }
}
