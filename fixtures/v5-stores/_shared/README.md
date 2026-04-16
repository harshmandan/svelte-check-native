# v5-stores fixtures

Locally-maintained Svelte 5 store-pattern fixtures, ported from
upstream `svelte2tsx`'s test corpus.

## Why these exist

The upstream `svelte2tsx` corpus has ~30 store-related fixtures, but
most are written in Svelte 4 syntax (`export let foo`, `$:` reactive
statements, `on:event` directives, `<slot>`, etc.). Since we declare
Svelte 5 only, we can't run those upstream fixtures verbatim — they'd
look like compile errors to us.

The fixtures in this directory are direct ports of the
store-pattern-relevant ones from upstream, with Svelte 4 surface
syntax mechanically rewritten to Svelte 5 (`on:click=` →
`onclick=`). The store usage itself (`$store`, `import { writable }
from 'svelte/store'`, etc.) is unchanged because Svelte's store
runtime is the same in v4 and v5.

Fixtures using `$:` reactive statements were NOT ported — they
require a manual rewrite to `$derived` / `$effect` runes that
isn't a mechanical translation. Add them here individually as needed.

## Layout

```
fixtures/v5-stores/
├── _shared/
│   ├── tsconfig.base.json   shared base for every fixture
│   └── README.md            this file
└── <fixture-name>/
    └── input.svelte         the SUT
```

The runner is `crates/cli/tests/v5_stores_fixtures.rs` (uses the same
`run.cjs` as the upstream `v5_fixtures` test, just with a different
SAMPLES_DIR + BASELINES). Fixtures expected to produce errors go in
`v5_stores_fixtures/baselines.json` next to the runner.
