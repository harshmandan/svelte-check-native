// Simulated overlay emit for VirtualList.svelte (generic component).
//
// Source Svelte (conceptual):
//   <script lang="ts" generics="T">
//       import type { Snippet } from 'svelte';
//       let { items, children }: {
//           items: T[];
//           children: Snippet<[T]>;
//       } = $props();
//   </script>
//   {#each items as item}
//       {@render children(item)}
//   {/each}

import type { Snippet } from 'svelte';

async function $$render_virtual_list<T>() {
    let { items, children }: {
        items: T[];
        children: Snippet<[T]>;
    } = $props();

    async function __svn_tpl_check() {
        for (const item of __svn_each_items(items)) {
            void item;
            children(item);
        }
    }
    void __svn_tpl_check;
    void items;
    void children;
}
$$render_virtual_list;

// The callable's `<T>` is what lets TS infer the element type from the
// items prop at each consumer call site, which then flows into the
// snippet parameter via `Snippet<[T]>`. That's the whole reason we pick
// a call-signature shape for the default export instead of a typed
// const.
import { SvelteComponent as $$_SC } from 'svelte';

declare class __svn_component_default<T> extends $$_SC<{
    items: T[];
    children: Snippet<[T]>;
}> {
    constructor(options: { target?: any; props?: Partial<{
        items: T[];
        children: Snippet<[T]>;
    }> });
}

export default __svn_component_default;
