// Every way a `<svelte:self>` instance is used in the template check.
let element: unknown;
let typed!: import('svelte').SvelteComponent;
export function render() {
    // `<svelte:self prop1={1} />`
    { __svn_create_component_any({ prop1: 1 }); }
    // `<svelte:self on:foo />` (and `on:foo={handler}`)
    { const inst = __svn_create_component_any({}); inst.$on('foo', () => {}); }
    // `<svelte:self let:prop>…</svelte:self>`, with implicit children
    {
        const inst = __svn_create_component_any({ children: () => __svn_snippet_return() });
        { const { prop } = inst.$$slot_def.default; prop; }
    }
    // `<svelte:self bind:this={element} />`
    { const inst = __svn_create_component_any({}); element = inst; typed = inst; }
    return [element, typed];
}
