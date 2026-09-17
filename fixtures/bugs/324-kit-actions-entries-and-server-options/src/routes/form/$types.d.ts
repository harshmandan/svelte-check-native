// Stand-in for `svelte-kit sync`'s generated $types.d.ts for this route.
export type RequestEvent = { request: Request; url: URL; params: { id: string } };
export type PageServerLoadEvent = RequestEvent;
export type MaybePromise<T> = T | Promise<T>;
export type Actions = Record<string, (event: RequestEvent) => MaybePromise<void | Record<string, any>>>;
export type EntryGenerator = () => Array<{ id: string }>;
