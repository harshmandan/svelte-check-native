// Simulates the upstream emit shape that `__sveltets_2_fn_component`
// produces — i.e. just `import('svelte').Component<P, X, B>`.
//
// Selected by upstream when: Svelte-5 + runes mode + no slots + no events.
// Instance.svelte fits this profile, which is why upstream's overlay
// types it as `Component<InstanceProps, {}, ''>` not `$$IsomorphicComponent`.

interface InstanceProps {
    id: string;
    visible?: boolean;
}

export const InstanceComponent: import('svelte').Component<
    InstanceProps,
    {},
    ''
> = null as any;

export type InstanceComponentProps = InstanceProps;
