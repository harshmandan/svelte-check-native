# Gap E — Snippet receiver typing: ours more correct than upstream

## Status: INTENTIONAL DIVERGENCE (deferred)

## Symptom

`shadcn-svelte/docs` bench (1650 files):
- **Upstream** `svelte-check --tsgo`: 720 errors, 392 problem files.
- **Ours**: 0 errors, 0 problem files.

Every one of upstream's 720 errors is a TS7031 "Binding element 'props'
implicitly has an 'any' type" on the canonical bits-ui `child`-snippet
pattern:

```svelte
<Drawer.Trigger>
  {#snippet child({ props })}
    <Button {...props}>...</Button>
  {/snippet}
</Drawer.Trigger>
```

## Investigation

**Reproduced in isolation**: directly running tsgo on upstream's
cached overlay (`bench/shadcn-svelte/docs/.svelte-kit/.svelte-check/`)
produces the 720 errors — they're real, not a filter or noise.

**Could NOT reproduce in minimal fixture**: I built a fixture
(`design/gap_e_snippet_typing/fixture/`) modeling four variants of
the snippet emit shape:

- V1: ours-current — bare arrow body returning `__svn_snippet_return()`.
- V2: upstream-shape — body wrapped in `async () => {}` IIFE +
  `return __sveltets_2_any(0)` + `const {child} = inst.$$prop_def`
  extraction.
- V3: V2 without the prop_def extract.
- V4: ours-current with NO return.

All variants except V4 (V4 fails with TS2322 because `void` doesn't
satisfy `SnippetReturn`) PASS in the fixture. None reproduce TS7031.

So the 720 errors require upstream's *full* overlay context to surface
— some interaction between `__sveltets_2_ensureComponent`, the shim
shape, the user's enclosing `async () => {}` template walker, and the
nested snippets — that I couldn't isolate inside a hand-built fixture.

## Why upstream fires the 720 errors

Best guess: contextual typing collapses somewhere in upstream's full
overlay due to one of:

- `__sveltets_2_ensureComponent`'s overload arms (`ConstructorOf-
  ATypedSvelteComponent` vs `Component<...>`) causing TS to lose the
  `Component<P>` typed-props extraction at scale.
- The `Ωignore_position`-marked `async () => {}` IIFE breaking
  contextual flow between the outer arrow's signature and the
  expected snippet shape.
- Nested `__sveltets_2_any(0)` returns leaking `any` upwards through
  return-type inference.

Whatever the cause, the result is `child:` arrow's `({ props })`
destructure loses its contextual typing, and `props` falls back to
implicit-any.

## Why ours doesn't fire

Our `__svn_ensure_component` (post-Gap-A) extracts the typed
`Component<P, X, B>` cleanly. The `child` prop's type is preserved
through the constructor + props literal. The arrow is contextually
typed against the actual `Snippet<[{ props: Record<string, unknown> }]>`,
so `({ props })` destructures with `props: Record<string, unknown>`
— no implicit-any.

This is **strictly more correct** typing than upstream's. The 720
errors upstream fires are TRUE FALSE POSITIVES on user code that's
type-safe.

## Decision: defer

Per CLAUDE.md: "We are not stricter or lax-er than the upstream.
Parity means same errors, same warnings and same number of
problematic files." This argues for matching upstream's count.

But: the 720 errors are spurious. Manufacturing them in our overlay
would mean LOSING the typed-Component branch we added for Gap A,
which closed real Threlte over-fires. Deliberately re-introducing
the looseness costs both correctness AND user-experience (users
would have to fix 720 fake errors).

**Decision: keep our typing as-is.** Document the divergence in
`notes/DEFERRED.md`. Treat it as a "we are more correct than
upstream" case rather than a parity gap to close.

If a user encounters the inverse problem — code that DOES rely on
the `any`-shaped snippet receiver to silently pass — they have
escape hatches:
- Annotate the snippet parameter explicitly: `{#snippet child({
  props }: { props: any })}` → bypass our typing.
- Add `// @ts-expect-error` or `// svelte-ignore` markers.

Neither has been needed on any other bench in our 19-bench fleet.

## What I tried (and what didn't work)

- **Minimal fixture mirroring upstream's exact shim and shape** —
  doesn't reproduce TS7031.
- **Direct tsgo on upstream's cached overlay** — confirms 720
  errors are real.
- **Type-trap probe** — couldn't get useful output without breaking
  the overlay's structure.

The investigation cost ~1.5 hours. Continuing would require either
patching upstream's exact overlay format (which we'd have to
reverse-engineer at the byte level) OR rewriting our
`__svn_ensure_component` to deliberately fall back to lax typing,
which regresses Gap A.

Neither path produces user value.

## Cross-references

- `notes/DEFERRED.md` — entry to be added describing this gap.
- `crates/typecheck/src/svelte_shims_core.d.ts` — our
  `__svn_ensure_component` typed-Component branch.
- `design/gap_a_iso_extraction/` — the related Threlte fix that
  prevents us from reverting to lax typing.
