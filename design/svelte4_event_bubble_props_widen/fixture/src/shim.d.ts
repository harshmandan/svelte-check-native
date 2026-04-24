declare module 'svelte' {
    export interface Component<
        Props extends Record<string, any> = {},
        Exports extends Record<string, any> = {},
        Bindings extends keyof Props | '' = string
    > {
        (internals: unknown, props: Props): any;
    }
    export class SvelteComponent<P = any> {
        constructor(options: { target?: any; props?: P });
    }
    export interface ComponentConstructorOptions<Props> {
        target?: any;
        props?: Props;
    }
}
