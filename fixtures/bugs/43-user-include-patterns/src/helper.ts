export const greeting: string = 'hello';

// This file is reachable via the user's `include: ["src/**/*.ts"]` and
// is also imported by Foo.svelte. Both paths must put it in the
// overlay's program so it gets type-checked.
