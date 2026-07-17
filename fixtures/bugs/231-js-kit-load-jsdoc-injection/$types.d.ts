// Stand-in for `svelte-kit sync`'s generated $types.d.ts (same trick
// as fixtures 55/56/100). The JSDoc `@param` the kit-inject pass adds
// to an untyped JS `load` references `import('./$types.js').PageLoadEvent`;
// bundler module resolution lands on this sibling file.

export type RouteParams = { slug: string };

export type PageLoadEvent = {
    params: RouteParams;
    fetch: typeof fetch;
    url: URL;
};
