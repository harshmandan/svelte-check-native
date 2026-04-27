// Models a bits-ui-style component (Drawer.Trigger) whose `child` prop
// is a Snippet receiver. The pattern is exactly:
//   child?: Snippet<[{ props: Record<string, unknown> }]>
//
// We declare the Component<P> default-export shape that `.svelte`
// overlay would produce.

import type { Snippet, Component } from 'svelte';

export interface TriggerProps {
    child?: Snippet<[{ props: Record<string, unknown> }]>;
    children?: Snippet;
    disabled?: boolean;
}

export const Trigger: Component<TriggerProps, {}, ''> = null as any;
