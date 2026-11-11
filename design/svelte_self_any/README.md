# `svelte_self_any/`

Locks the `<svelte:self>` instantiation shape: svelte2tsx's
`__sveltets_2_createComponentAny({ props })`, an untyped
`SvelteComponent<any, any, any>` (InlineComponent.ts), which our emit
writes as `__svn_create_component_any` (declared as an alias of it).
`clean.ts` covers every use the template check makes of the instance:
plain props, `$on`, `$$slot_def` for `let:`, implicit `children`, and
`bind:this`.

`svelte` has to resolve:

```sh
ln -s /path/to/a/workspace/node_modules design/svelte_self_any/node_modules
tsgo -p design/svelte_self_any/tsconfig.json
```

Expected: `clean.ts` silent; `broken.ts` firing TS2345 (non-object props)
and TS2322 (instance assigned to `string`).
