# Shared fixture helpers

## `tsconfig.base.json`

Minimal tsconfig inherited by every bug fixture. Bug fixtures extend this via:

```jsonc
{
    "extends": "../_shared/tsconfig.base.json",
    "include": ["**/*"]
}
```

Keep this file **minimal and strict** — the point of the fixtures is to
catch bugs that only surface under strict checking (`noUnusedLocals`,
`strict`, `noUnusedParameters`).

## Fixture naming

`fixtures/bugs/<NN>-<kebab-case-slug>/` where `NN` is a two-digit zero-padded
number matching the bug in `todo.md` Phase 2.1. The order is historical — it's
the order bugs were discovered in the `upstream` rescue.

## Fixture contents

Each fixture directory contains:

- `input.svelte` (or `input.svelte.ts`) — minimal reproduction
- `tsconfig.json` — usually 2 lines: `extends` + `include`
- `expected.json` — either `{"clean": true}` or `{"errors": [...]}` with
  the exact expected diagnostics
- optional `README.md` — one-paragraph explainer linking back to
  `todo.md` or `upstream/report.md` (reference only)

The runner in `crates/cli/tests/bug_fixtures/run.cjs` iterates this dir.
