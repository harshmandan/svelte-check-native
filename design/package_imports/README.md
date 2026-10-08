# `package_imports/`

Locks how a `.svelte` file imported through a package.json `imports`
specifier (`#lib/Comp.svelte`, SvelteKit 3's replacement for `$lib`)
reaches the type file the overlay generates for it.

`cache/` stands in for the overlay: `cache/svelte/` holds the generated
`Comp.d.svelte.ts`, and `ambient.d.ts` is svelte's `declare module
'*.svelte'` wildcard. TypeScript resolves `#lib/*` through package.json
to `src/lib/Comp.svelte`, where no type file exists, so the import
falls through to the wildcard. `rootDirs` does not help: it applies to
relative imports only. Overlay `paths` entries that list the source
directory and its generated mirror do, because `paths` is consulted
before package.json `imports`.

```sh
tsgo -p design/package_imports/cache/tsconfig.nopaths.json  # today
tsgo -p design/package_imports/cache/tsconfig.json          # with paths
```

Expected without `paths`: TS2614 `Module '"*.svelte"' has no exported
member` for every named import from a `.svelte` file, `clean.ts`
included. With `paths`: `clean.ts` silent; `broken.ts` firing TS2614
(`Missing` from `"#lib/Comp.svelte"`) at 2:15 and TS2322 at 4:7 and 5:7.
