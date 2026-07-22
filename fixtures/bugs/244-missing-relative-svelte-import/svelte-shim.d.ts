// Stand-in for svelte's own `declare module '*.svelte'` (shipped in
// `svelte/types/index.d.ts`). Its presence makes tsgo resolve any
// `.svelte` specifier to `any`, so the missing-import error must come
// from our native detection, not tsgo.
declare module '*.svelte' {
  const component: new (...args: any[]) => any
  export default component
}
