// Minimal stand-in for the `svelte` module so the `import('svelte').Component<P>`
// JSDoc references in the JS overlays resolve. Production overlay reuses the
// real svelte package's d.ts; we only need a one-shape stub for the design
// fixture.

declare module 'svelte' {
    export interface Component<Props extends Record<string, any> = any> {
        (anchor: any, props: Props): any;
    }
}
