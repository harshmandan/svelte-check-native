// Edge cases to confirm the callable shape's robustness:
//   (a) bind:this with `HTMLInputElement | undefined` variable type
//   (b) component whose $props() is untyped (Record<string, any>)
//   (c) generic component with explicit type argument at call site
//   (d) snippet child passed through a wrapper as a prop reference
// Zero errors expected in this file.

import Switch from './Switch.svelte.ts';
import VirtualList from './VirtualList.svelte.ts';
import Wrapper from './Wrapper.svelte.ts';
import type { Snippet } from 'svelte';

// Untyped-props component: simulate an overlay whose Props is `Record<string, any>`.
declare function __Untyped(__anchor: any, props: Record<string, any>): any;

async function $$render_edges() {
    // (a) bind:this target typed `HTMLInputElement | undefined` — must pass.
    let elA: HTMLInputElement | undefined;
    __svn_bind_this_check<HTMLInputElement>(elA);
    void elA;

    //     bind:this target typed `HTMLInputElement` (non-null) — must pass.
    //     Definite-assignment `!` is what our existing
    //     rewrite_definite_assignment_in_place pass injects into the
    //     user's `let`; simulated here.
    let elB!: HTMLInputElement;
    __svn_bind_this_check<HTMLInputElement>(elB);
    void elB;

    // (b) Untyped-props component: silently accepts anything. No error.
    __Untyped(__svn_any(), { random: 1, stuff: 'ok' });

    // (c) Generic component with explicit type argument — rare but legal.
    type Item = { id: number; title: string };
    const items: Item[] = [{ id: 1, title: 'a' }];
    VirtualList<Item>(__svn_any(), {
        items,
        children: (item) => {
            // `item` should be Item — test field access.
            void item.id;
            void item.title;
            return __svn_snippet_return();
        },
    });

    // (d) Snippet passed through a user-defined variable.
    const rowRef: Snippet<[{ id: number; label: string }]> = ({ id, label }) => {
        void id;
        void label;
        return __svn_snippet_return();
    };
    Wrapper(__svn_any(), {
        items: [{ id: 1, label: 'a' }],
        row: rowRef,
    });
    void Switch;
}
$$render_edges;

declare const __svn_component_default: any;
export default __svn_component_default;
