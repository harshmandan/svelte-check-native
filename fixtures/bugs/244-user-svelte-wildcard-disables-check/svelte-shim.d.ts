// A USER-authored `declare module '*.svelte'` wildcard (not svelte's own,
// which lives in node_modules). The default `svelte-check` keeps this, so
// it resolves every `.svelte` import; our ambient guard detects it and
// disables the missing-import check to match. tsgo also resolves the
// import through this wildcard, so the whole run stays clean.
declare module '*.svelte' {
  const component: new (...args: any[]) => any
  export default component
}
