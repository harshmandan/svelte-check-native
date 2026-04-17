// Stand-in for `svelte-kit sync`'s generated $types.d.ts. Route emit
// injects `import('./$types.js').PageData` into untyped `$props()`
// destructures; TS's bundler module resolution rewrites `./$types.js`
// to this sibling file.

export type PageData = {
    title: string;
    count: number;
};

export type LayoutData = {};
export type ActionData = undefined;
