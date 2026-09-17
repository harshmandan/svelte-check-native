// Stand-in for the `$types.d.ts` that `svelte-kit sync` generates.
export type PageData = { title: string };
export type LayoutData = { title: string };
export type ActionData = undefined;
export type PageProps = { data: PageData; params: { slug: string } };
export type LayoutProps = { data: LayoutData; params: {} };
export type Snapshot<T = any> = { capture: () => T; restore: (snapshot: T) => void };
