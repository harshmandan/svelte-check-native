// Minimal inline copy of the svelte types our fixtures need. Keeps
// the fixtures runnable against a bare tsgo install without pulling
// the real svelte package as a dependency.
//
// Shapes cribbed from upstream svelte-shims.d.ts (the real package)
// and our own svelte_shims_core.d.ts fallback block. Kept minimal
// deliberately — if a fixture needs more, add here with a one-line
// comment pointing at the real-world consumer.

declare module 'svelte' {
    // Real svelte uses `Record<string, any>` (loose) — cribbed from
    // bench/*/node_modules/svelte/types/index.d.ts. Keep loose so
    // fixture compiles without spurious TS2344 "doesn't satisfy
    // Record<string, unknown>" on interfaces without explicit index
    // signatures.
    export interface ComponentConstructorOptions<
        Props extends Record<string, any> = Record<string, any>,
    > {
        target: Element | Document | ShadowRoot;
        anchor?: Element;
        props?: Props;
        context?: Map<any, any>;
        hydrate?: boolean;
        intro?: boolean;
    }

    export class SvelteComponent<
        Props extends Record<string, any> = Record<string, any>,
        Events extends Record<string, any> = Record<string, any>,
        Slots extends Record<string, any> = Record<string, any>,
    > {
        constructor(options: ComponentConstructorOptions<Props>);
        $set(props: Partial<Props>): void;
        $destroy(): void;
        $$prop_def: Props;
        $$events_def: Events;
        $$slot_def: Slots;
    }

    export type Component<
        Props extends Record<string, any> = Record<string, any>,
        Exports extends Record<string, any> = Record<string, any>,
        Bindings extends keyof Props | '' = string,
    > = (...args: any[]) => {
        props: Props;
        exports: Exports;
        bindings: Bindings;
    };

    export type ComponentProps<T> =
        0 extends 1 & T ? any :
        T extends Component<infer Props, any, any> ? Props :
        T extends SvelteComponent<infer Props, any, any> ? Props :
        T extends new (...args: any[]) => SvelteComponent<infer Props, any, any>
            ? Props :
        any;
}
