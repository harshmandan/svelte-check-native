// Minimal inline svelte types + the get/set binding helper. Keeps the
// fixtures runnable against a bare tsgo install without pulling the real
// svelte package as a dependency.
//
// Helper shape mirrors upstream svelte-shims-v4.d.ts:269 exactly —
// `__sveltets_2_get_set_binding<T>(get, set): T`. Our port uses an
// `__svn_*` prefix per CLAUDE.md architecture rule #6. Identical
// semantics.

declare module 'svelte' {
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
}

// The helper itself. Lives at module scope in both upstream and our
// overlay (see `crates/typecheck/src/svelte_shims_core.d.ts`).
declare function __svn_get_set_binding<T>(
    get: (() => T) | null | undefined,
    set: (t: T) => void,
): T;
