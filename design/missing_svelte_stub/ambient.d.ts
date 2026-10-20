// Stands in for svelte's own types: a `Component` type and the
// `*.svelte` wildcard that resolves every `.svelte` specifier, missing
// ones included.
declare module 'svelte' {
    export type Component<Props extends Record<string, any> = {}> = (
        internals: unknown,
        props: Props
    ) => Record<string, any>;
}

declare module '*.svelte' {
    import { Component } from 'svelte';
    const component: Component<any>;
    export default component;
}
