declare module 'svelte' {
    export type Snippet<Parameters extends any[] = []> = {
        (...args: Parameters): any;
    };
    export type Component<
        Props extends Record<string, any> = Record<string, any>,
    > = (...args: any[]) => { props: Props };
    export class SvelteComponent<
        Props extends Record<string, any> = Record<string, any>,
    > {
        constructor(options: { target?: any; props?: Props });
        $set(props: Partial<Props>): void;
        $$prop_def: Props;
    }
}
