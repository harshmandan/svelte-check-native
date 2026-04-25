<div align="center">

  <img src="./svelte-check-native.webp" alt="svelte-check-native" width="300" />

  <h1>svelte-check-native</h1>

[![version](http://img.shields.io/npm/v/svelte-check-native.svg)](https://www.npmjs.com/package/svelte-check-native)
[![downloads](http://img.shields.io/npm/dm/svelte-check-native.svg)](https://www.npmjs.com/package/svelte-check-native)

</div>

Blazing fast CLI type-checker for **Svelte** projects.
Drop-in replacement for [`svelte-check`](https://www.npmjs.com/package/svelte-check) — compatible flags, byte-identical output, same exit codes. Powered by Rust + [tsgo](https://github.com/microsoft/typescript-go). Made for:

- AI agents that need tight feedback loops
- Fast CI/CD pipelines
- Git pre-commit hooks that actually stay enabled

| What it is                                  | What it isn't               |
| ------------------------------------------- | --------------------------- |
| CLI type-checker for Svelte 4 and 5         | An LSP / editor integration |
| Drop-in for `svelte-check` (flags + output) | A CSS linter                |
| Single Rust binary, tsgo-powered            | A formatter                 |
| Byte-identical diagnostics upstream         |                             |
| Incremental via tsgo's `tsbuildinfo`        |                             |

## Speed

Measured on a SvelteKit + TypeScript monorepo with
1359 `.svelte` files (Svelte 5 runes), M1 Pro 8C, mean of 2 runs each:

`svelte-check-native --tsconfig tsconfig.json --diagnostic-sources 'js,svelte'`

```
  tool                  cold     warm     dirty   speedup   errors/warnings/problems
──────────────────────────────────────────────────────────────────────────────────────
svelte-check-native      2.4s     1.0s     0.9s      41x           0/49/17
svelte-check            40.0s    41.0s    41.6s     1.0x           0/49/17
svelte-check --tsgo     18.3s    18.6s    17.9s     2.9x           1/49/18
svelte-check-rs         12.2s     5.5      4.4s     7.5x         732/44/261
```

Diagnostic counts match `svelte-check` with same flags.

## Install

```sh
npm i -D svelte-check-native @typescript/native-preview
```

`@typescript/native-preview` is the tsgo binary — required at check
time, never imported at runtime.

## Use

```sh
npx svelte-check-native --workspace .
```

Or add it to `package.json`:

```json
{
  "scripts": {
    "check": "svelte-check-native --workspace ."
  }
}
```

Same flags as `svelte-check`. See `npx svelte-check-native --help`.

## How it works

Single Rust binary. Each `.svelte` file flows through a handful of
crates in one process:

| Crate             | What it does                                                                                    |
| ----------------- | ----------------------------------------------------------------------------------------------- |
| `parser`          | Parses `.svelte` source into a Svelte-5 AST (script + template).                                |
| `analyze`         | Builds a `SemanticModel` — runes, prop shapes, bindings, scope — used by emit and lint.         |
| `emit`            | Generates the `.svelte.ts` overlay tsgo will type-check. Imports rewritten so tsgo lands on it. |
| `svn-lint`        | Native Rust port of `svelte/compiler`'s warning pass. Covers all known codes; no subprocess.    |
| `svelte-compiler` | Fallback bridge to the user's `svelte/compiler` over a persistent `bun`/`node` worker pool.     |
| `typecheck`       | Owns the tsgo overlay tsconfig + the `tsbuildinfo` cache; invokes tsgo; maps diagnostics back.  |
| `core`            | Shared types (spans, diagnostics, position maps) + the canonical `TsConfig` struct.             |
| `cli`             | Entrypoint. Flag parsing, file discovery, output formatting, exit codes.                        |

## Flags

Every flag not listed below behaves the same as `svelte-check`.

### New flags

```
--svelte-warnings <mode>      How to source Svelte compiler warnings:
                              native | bridge
                              native: rust port faster
                              bridge: js bridge same as svelte-check, slower by 1.5-2s
--timings                     Phase-by-phase wall-clock breakdown
--debug-paths                 Print resolved binaries, exit
--tsgo-version                Print tsgo version, exit
--tsgo-diagnostics            Print tsgo's perf/memory stats after the run
--emit-ts                     Print generated TypeScript per file, exit
```

### Not supported

- `--watch` / `--preserveWatchOutput` — use [`watchexec`](https://github.com/watchexec/watchexec)
  or your editor's file watcher externally.
- `--no-tsconfig` — errors out. A tsconfig is required.
- `--incremental` — always on. tsgo's `tsbuildinfo` handles it.
- `--tsgo` — always on. Classic `tsc` is not wired up.
- `--diagnostic-sources css` — accepted but no-op (a CSS language
  service isn't bundled). Roadmap below.

Run `svelte-check-native --help` for the full list.

Output defaults to `machine` when run from a coding-agent CLI:
`CLAUDECODE=1` (Claude Code), `GEMINI_CLI=1` (Gemini CLI), or `CODEX_CI=1` (OpenAI Codex CLI).

## Environment variables

- `TSGO_BIN` — override tsgo discovery; accepts an absolute path to a
  platform-native tsgo binary. Useful when `@typescript/native-preview`
  isn't in `node_modules` (e.g. a monorepo where tsgo lives elsewhere).
- `SVN_BRIDGE_WORKERS` — number of `svelte/compiler` worker
  subprocesses. Default `cores/2`, capped at 8; tracks the perf-core
  count on Apple Silicon. Override if you hit IPC contention on very
  large core counts.
- `CLAUDECODE` / `GEMINI_CLI` / `CODEX_CI` — any set forces `machine`
  output for agent-friendly parsing.

## Exit codes

- `0` — no errors (and no warnings if `--fail-on-warnings`)
- `1` — errors detected (or warnings with `--fail-on-warnings`)
- `2` — invocation error (bad flag, missing tsconfig, tsgo not found)

## Monorepos

Pointed at a pnpm/npm/yarn workspace root with no `paths` of its own
(`{ "extends": "tsconfig/base.json" }` and a `pnpm-workspace.yaml`
listing `apps/*` / `packages/*`), this binary auto-discovers each
member's `tsconfig.json` and unions their `compilerOptions.paths`
into the overlay. So `import x from '$lib/foo'` from
`apps/dashboard/src/...` resolves to `apps/dashboard/src/lib/foo`,
and the same alias from `apps/api/src/...` resolves to
`apps/api/src/lib/foo` — without restructuring your tsconfig.

Tradeoff: when two members declare the SAME alias targeting
DIFFERENT directories (`$lib` in `apps/a` vs `apps/b`), tsgo tries
each in order. As long as module names don't collide across
sub-apps the right one resolves; cross-app name collisions silently
misresolve to the first-listed sub-app. Acceptable v0 — real
monorepos rarely overlap module names across `$lib` trees.

This is an innovation beyond upstream `svelte-check`. Their
`--tsgo` mode requires `--tsconfig` and refuses to auto-discover.
For tighter parity (and predictable per-app diagnostics), invoke
this binary per sub-app instead:

```sh
svelte-check-native --workspace apps/dashboard
```

## Roadmap

- [ ] **CSS lint diagnostics**

## Troubleshooting

**Stale errors after editing `tsconfig.json` or path aliases** — wipe
the cache: `rm -rf node_modules/.cache/svelte-check-native` and re-run.
The overlay config is regenerated from your live tsconfig on every
run, but tsgo's `tsbuildinfo` can hold onto stale resolution state.

**TS2321 "Excessive stack depth"** on your own types — usually a
`UnionToRecord<T>` that round-trips through `UnionToTuple<T>[number]`.
Iterate `T` directly in the mapped-type key instead.

## Prior art

- [`svelte2tsx`](https://github.com/sveltejs/language-tools/tree/master/packages/svelte2tsx)
  / [`svelte-check`](https://github.com/sveltejs/language-tools/tree/master/packages/svelte-check)
  — transpiler + CLI whose output shape and flags we
  match. The `.v5` fixture corpus from `svelte2tsx` is our parity gate.
- [tsgo](https://github.com/microsoft/typescript-go) — the Go-based
  TypeScript compiler that does the actual type-checking. Shipped as
  `@typescript/native-preview`.

## License

MIT
