# `missing_svelte_stub/`

Locks the stub declaration the overlay writes for a `.svelte` import
that resolves to no file (TSGO-ENHANCEMENT layer).

The default svelte-check engine blanks svelte's `declare module
'*.svelte'` block when it loads svelte's types
(`language-server/src/plugins/typescript/DocumentSnapshot.ts`,
`fromNonSvelteFilePath`), so a missing module stays unresolved: TS2307
at each reference and an `any`-like error type for every use — value,
type, namespace member, generic, qualified name — with nothing else
reported. Under tsgo that wildcard (`ambient.d.ts` here) resolves the
import instead, and its value-only `Component<any>` default with no
named exports fires TS2614, TS2749, TS2694, TS2339, TS7053, TS2322,
TS2554 and TS6196.

The stub is a comment-only declaration file: a script, not a module.
The compiler resolves the import to it, reports TS2306 "File '…' is not
a module" at the specifier and then treats the import exactly as
unresolved — the wildcard is never consulted. The pipeline drops each
TS2306 that names a stub (its own TS2307 stands in its place).

A relative target is reached through `rootDirs` (the stub sits at the
target's `.d.svelte.ts` mirror path); an aliased or bare specifier
through an exact-match `paths` entry, which wins over wildcard patterns
like `$lib/*`.

```sh
tsgo -p design/missing_svelte_stub/cache/tsconfig.nostub.json  # today
tsgo -p design/missing_svelte_stub/cache/tsconfig.json         # with stubs
```

Expected without stubs: `clean.ts` fires the wildcard errors listed
above. With stubs: only TS2306 at each `.svelte` specifier (1:42, 2:21,
3:27, 4:23, 5:18, 13:25, 17:52, 18:15, 19:22, 21:26 in `clean.ts`;
1:18 in `broken.ts`), and `broken.ts`'s unrelated TS2322 at 3:14, which
the stub must not hide.
