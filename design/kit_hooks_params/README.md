# `kit_hooks_params/`

Locks the annotation shape `kit_inject` splices into SvelteKit hooks
files and param matchers. The `clean.ts` / `broken.ts` contents were
copied from a real overlay upstream `svelte-check --tsgo` wrote for the
same inputs, not hand-derived, so the spacing and the
`Parameters<T>[0]` / `ReturnType<T>` projection are upstream's exactly.

`@sveltejs/kit` has to resolve for these to compile. Point the fixture
at a workspace that has it installed:

```sh
ln -s /path/to/a/kit/app/node_modules design/kit_hooks_params/node_modules
tsgo --noEmit -p design/kit_hooks_params/tsconfig.json
```

Expected: `clean.ts` silent, `broken.ts` firing TS2322 twice, TS2339
once, and TS1064 once. That last one is not a mistake in the fixture —
see the comment above it.
