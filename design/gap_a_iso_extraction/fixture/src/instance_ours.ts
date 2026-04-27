// Simulates the .svelte-check overlay we currently emit for `Instance.svelte`.
//
// `$$IsomorphicComponent` callable RETURN includes `& { $set?: any; $on?: any }`.
// This is the v0.5+1 shape (commit fd126e98).

interface InstanceProps {
    id: string;
    visible?: boolean;
}

async function $$render_ours() {
    return {
        props: undefined as any as InstanceProps,
        events: undefined as any as { [evt: string]: CustomEvent<any> },
        slots: undefined as any as {},
        bindings: undefined as any as string,
        exports: undefined as any as {},
    };
}

declare class __svn_Render_ours {
    props(): Awaited<ReturnType<typeof $$render_ours>>['props'];
    events(): Awaited<ReturnType<typeof $$render_ours>>['events'];
    slots(): Awaited<ReturnType<typeof $$render_ours>>['slots'];
    bindings(): Awaited<ReturnType<typeof $$render_ours>>['bindings'];
    exports(): Awaited<ReturnType<typeof $$render_ours>>['exports'];
}

interface $$IsomorphicComponentOurs {
    new (
        options: import('svelte').ComponentConstructorOptions<
            ReturnType<__svn_Render_ours['props']>
        >,
    ): import('svelte').SvelteComponent<
        ReturnType<__svn_Render_ours['props']>,
        ReturnType<__svn_Render_ours['events']>,
        ReturnType<__svn_Render_ours['slots']>
    > & { $$bindings?: ReturnType<__svn_Render_ours['bindings']> } &
        ReturnType<__svn_Render_ours['exports']>;
    (
        internal: unknown,
        props: ReturnType<__svn_Render_ours['props']>,
    ): ReturnType<__svn_Render_ours['exports']> & { $set?: any; $on?: any };
    z_$$bindings?: ReturnType<__svn_Render_ours['bindings']>;
}

export const InstanceOurs: $$IsomorphicComponentOurs = null as any;
export type InstanceOurs = InstanceType<typeof InstanceOurs>;
export type InstanceOursProps = InstanceProps;
