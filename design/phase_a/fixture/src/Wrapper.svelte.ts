// Simulated overlay emit for Wrapper.svelte.
//
// Source Svelte (conceptual):
//   <script lang="ts">
//       import type { Snippet } from 'svelte';
//       let { items, row }: {
//           items: Array<{ id: number; label: string }>;
//           row: Snippet<[{ id: number; label: string }]>;
//       } = $props();
//   </script>
//   {#each items as item}
//       {@render row(item)}
//   {/each}

import type { Snippet } from 'svelte';

type WrapperProps = {
    items: Array<{ id: number; label: string }>;
    row: Snippet<[{ id: number; label: string }]>;
};

async function $$render_wrapper() {
    let { items, row }: WrapperProps = $props();

    async function __svn_tpl_check() {
        for (const item of __svn_each_items(items)) {
            void item;
            row(item);
        }
    }
    void __svn_tpl_check;
    void items;
    void row;
}
$$render_wrapper;

import { SvelteComponent as $$_SC } from 'svelte';

declare class __svn_component_default extends $$_SC<WrapperProps> {
    constructor(options: { target?: any; props?: Partial<WrapperProps> });
}
export default __svn_component_default;
