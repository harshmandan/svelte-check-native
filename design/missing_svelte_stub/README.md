# `missing_svelte_stub/`

Locks the stub declaration the overlay writes for a `.svelte` import
that resolves to no file (TSGO-ENHANCEMENT layer).

The default svelte-check engine leaves such an import unresolved: it
reports TS2307 and every binding from it is `any`, in value AND type
position, so nothing else fires. Under tsgo, svelte's `declare module
'*.svelte'` wildcard (`ambient.d.ts` here) resolves it instead, and the
bindings take the wildcard's shape: a value-only `Component<any>`
default and no named exports. That produces TS2614, TS2749, TS2339,
TS2322, TS2554 and TS6196 that the default engine never reports.

`cache/` stands in for the overlay. Each missing target gets a stub
that exports `any` in both spaces under every name its importers use:

```ts
declare const __svn_missing: any;
type __svn_missing = any;
export default __svn_missing;
export { __svn_missing as Named, __svn_missing as thing };
```

A relative target is reached through `rootDirs` (the stub sits at the
target's `.d.svelte.ts` mirror path); an aliased or bare specifier
through an exact-match `paths` entry, which wins over wildcard patterns
like `$lib/*`.

```sh
tsgo -p design/missing_svelte_stub/cache/tsconfig.nostub.json  # today
tsgo -p design/missing_svelte_stub/cache/tsconfig.json         # with stubs
```

Expected without stubs: `clean.ts` fires TS2614 ×3, TS6196, TS2749 ×3,
TS2322 ×2, TS2554 and TS2339. With stubs: `clean.ts` silent;
`broken.ts` fires TS2614 at 1:16 (a name the stub does not list — the
real overlay lists every imported name) and TS2322 at 4:14 (an
unrelated error the stubs must not hide), in both runs.
