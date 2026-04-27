// Simulates upstream's overlay shape — `addComponentExport.ts:170-179`.
//
// Per-component `$$IsomorphicComponent` callable RETURN does NOT include
// `& { $set?: any; $on?: any }`.

interface InstanceProps {
    id: string;
    visible?: boolean;
}

async function $$render_up() {
    return {
        props: undefined as any as InstanceProps,
        events: undefined as any as { [evt: string]: CustomEvent<any> },
        slots: undefined as any as {},
        bindings: undefined as any as string,
        exports: undefined as any as {},
    };
}

declare class __svn_Render_up {
    props(): Awaited<ReturnType<typeof $$render_up>>['props'];
    events(): Awaited<ReturnType<typeof $$render_up>>['events'];
    slots(): Awaited<ReturnType<typeof $$render_up>>['slots'];
    bindings(): Awaited<ReturnType<typeof $$render_up>>['bindings'];
    exports(): Awaited<ReturnType<typeof $$render_up>>['exports'];
}

interface $$IsomorphicComponentUpstream {
    new (
        options: import('svelte').ComponentConstructorOptions<
            ReturnType<__svn_Render_up['props']>
        >,
    ): import('svelte').SvelteComponent<
        ReturnType<__svn_Render_up['props']>,
        ReturnType<__svn_Render_up['events']>,
        ReturnType<__svn_Render_up['slots']>
    > & { $$bindings?: ReturnType<__svn_Render_up['bindings']> } &
        ReturnType<__svn_Render_up['exports']>;
    (
        internal: unknown,
        props: ReturnType<__svn_Render_up['props']> & {},
    ): ReturnType<__svn_Render_up['exports']>;
    z_$$bindings?: ReturnType<__svn_Render_up['bindings']>;
}

export const InstanceUpstream: $$IsomorphicComponentUpstream = null as any;
export type InstanceUpstream = InstanceType<typeof InstanceUpstream>;
export type InstanceUpstreamProps = InstanceProps;
