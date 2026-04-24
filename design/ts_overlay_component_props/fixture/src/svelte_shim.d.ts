// Cut-down svelte ambient. Mirrors the pieces the overlay uses so
// the fixture can run without pulling in real `svelte` types.

declare module 'svelte' {
    export interface Component<
        Props extends Record<string, any> = {},
        Exports extends Record<string, any> = {},
        Bindings extends keyof Props | '' = string extends keyof Props ? string : '',
    > {
        (
            this: void,
            internals: unknown,
            props: Props,
        ): {
            $on?<Evt extends string>(
                type: Evt,
                callback: (e: any) => void,
            ): () => void;
            $set?(props: Partial<Props>): void;
        } & Exports;
        element?: typeof HTMLElement;
        z_$$bindings?: Bindings;
    }
}

declare function __svn_ensure_component<
    C extends new (...args: any[]) => any,
>(c: C): C;
declare function __svn_ensure_component<
    T extends import('svelte').Component<any, any, any>,
>(
    c: T,
): T extends import('svelte').Component<
    infer P extends Record<string, any>,
    any,
    any
>
    ? new (options: { target?: any; props?: P }) => {
          $on?(evt: string, fn: (e: any) => void): () => void;
          $set?(props: Partial<P>): void;
      }
    : never;
declare function __svn_ensure_component(
    c: unknown,
): new (options: { target?: any; props?: any }) => any;

declare function __svn_any(): any;
