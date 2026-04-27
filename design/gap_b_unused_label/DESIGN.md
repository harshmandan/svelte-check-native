# Gap B — TS7028 unused-label filter (validated 2026-04-27)

## Problem

`threlte/{flex,theatre}` over-fires +1/+3 errors on Svelte-4 reactive
statements:

> `Sequence.svelte:73:12 "Unused label."`
> `Sequence.svelte:98:12 "Unused label."`
> `Sequence.svelte:108:12 "Unused label."`
> `+page.svelte:55:12 "Unused label."`

Upstream is clean on these.

## Root cause

Both ours and upstream emit `$: <expr>` reactive statements as
`;() => { $: <expr> }` — an arrow IIFE that gives tsgo a body to
type-check without actually running it. The structural `$:` label
inside is the source of false-positive TS7028s under
`allowUnusedLabels: false` (set explicitly in threlte's tsconfig and
many other strict SvelteKit tsconfigs).

**Same emit shape, different diagnostic counts** because upstream's
svelte-check has a post-emit diagnostic filter:

> `language-tools/packages/language-server/src/plugins/typescript/
>  features/DiagnosticsProvider.ts:191`
>     `.filter(not(isUnusedReactiveStatementLabel))`

The filter walks each TS7028 diagnostic to its source AST node;
drops it if the parent is a reactive `$:` `LabeledStatement`. We
emit the same shape but don't have the filter, so all the `$:`
labels leak through.

## Validated fix shape

Run `tsgo --pretty false -p tsconfig.json` from this fixture:

- Without filter: 2 TS7028 errors (both `$:` labels).
- With filter (in `crates/typecheck/src/lib.rs::map_diagnostic`): 0
  errors.

## Implementation

In `map_diagnostic`, when `raw.code == 7028`, check the overlay byte
at the diagnostic offset. If it's `$` followed (optionally with
whitespace) by `:`, drop the diagnostic. The `$` identifier is the
exact char tsgo points at for TS7028 — by construction the only
single-char `$` label in our emit is the structural reactive label.

The byte check is faster and simpler than the upstream AST-walk
approach; same coverage in practice because we don't synthesize any
other `$`-named labels.

## Risk

- **False positives**: a user could write a literal `$:` label in
  hand-written TS embedded in their script. That's allowed JS syntax
  but no Svelte codebase actually does it (the `$:` is reserved by
  Svelte's syntax). Even if they did, the user's `$:` would also be
  unused, so dropping the warning is the same charity upstream
  extends.
- **Cross-bench regressions**: none. The filter only fires on the
  exact `$` identifier on a reactive label; everything else passes
  through.

## Validation gates

1. tsgo on this fixture: 2 TS7028 errors.
2. After implementation: re-run threlte/flex (3→2E byte-perfect) and
   theatre (9→6E, two of the +3 close).
3. Bench sweep: no regressions on the 11 byte-perfect benches.
