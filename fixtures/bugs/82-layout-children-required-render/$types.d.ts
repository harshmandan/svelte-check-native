// Stand-in for `svelte-kit sync`'s generated $types.d.ts. The layout
// emit injects `children: import('svelte').Snippet` into the
// synthesized `$$ComponentProps` for `+layout.svelte` files; this
// file isn't read directly here but its presence lets the
// `import('./$types.js')` resolution pattern work uniformly with
// other Kit-route fixtures.

export type LayoutData = {};
