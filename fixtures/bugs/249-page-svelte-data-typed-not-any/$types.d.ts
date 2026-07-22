// Stand-in for `svelte-kit sync`'s generated $types.d.ts. Our route emit
// injects `import('./$types.js').PageData` into the untyped `$props()`
// destructure; bundler resolution rewrites `./$types.js` to this sibling.
export type PageData = {
    title: string;
};
export type LayoutData = {};
export type ActionData = undefined;
