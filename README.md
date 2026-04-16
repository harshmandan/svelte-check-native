# svelte-check-native

A fast, CLI-only type checker for **Svelte 5+** projects. Written in
Rust, powered by [tsgo](https://github.com/microsoft/typescript-go) for
TypeScript diagnostics and the user's installed `svelte/compiler` for
compiler warnings.

Designed as a drop-in CLI replacement for
[`svelte-check`](https://www.npmjs.com/package/svelte-check) — same
flags, same output formats, same exit codes — restricted to Svelte 5+
syntax.

---

## Why it exists

`svelte-check` is the standard pre-commit / CI gate for Svelte projects.
On large codebases (1k+ components) it takes 30–60+ seconds with the
default tsc backend (or roughly 10–15 seconds with the experimental
`--tsgo` flag), blocks pushes, blocks deploys, and blocks AI agents
from iterating on generated code. The single-process Node startup, the
per-file svelte2tsx work, and tsc's full type-graph re-walk are the
bottlenecks.

`svelte-check-native` runs the same checks as `svelte-check` against
the same projects in roughly 5 seconds (warm) — about 8× faster than
`svelte-check`'s default mode, 2.5× faster than `svelte-check --tsgo`
— with byte-identical output for the formats CI tooling consumes.

It is built specifically for two consumers:

- **CI/CD pipelines.** Faster feedback, exit codes that mean what they
  say, no flaky `npm install` step for the checker itself once
  installed.
- **AI coding agents.** Sub-5-second feedback loops on file changes
  matter when an agent is writing-then-checking dozens of files in a
  session. Stable machine-verbose JSON for parsing, automatic switch to
  `machine` output under `CLAUDECODE=1`.

---

## Status

Working today, against real Svelte 5 codebases:

- Type-check parity with upstream `svelte-check --tsgo` on 1200-file
  SvelteKit projects (zero-error gap, identical compiler-warning set
  for Svelte-source warnings)
- 59/63 fixtures from `svelte2tsx`'s upstream test corpus pass
  (51 zero-error + 8 within-baseline; see [Test suite](#test-suite))
- 24/24 store-pattern fixtures pass
- All four output formats (`human`, `human-verbose`, `machine`,
  `machine-verbose`) byte-compatible with upstream

Not yet implemented: watch mode, CSS diagnostics, several debug flags.
See [Missing flags](#missing-flags).

---

## Installation

`svelte-check-native` is a single Rust binary plus a small JS bridge
(baked in). Build from source:

```sh
git clone https://github.com/harshmandan/svelte-check-native
cd svelte-check-native
cargo build --release
# binary lands at target/release/svelte-check-native
```

NPM distribution is planned for `v0.1.0`.

---

## Requirements

| Dependency | Why | How resolved |
|---|---|---|
| `@typescript/native-preview` (tsgo) | TypeScript diagnostics backend | Discovered in your project's `node_modules`. Install: `bun add -d @typescript/native-preview` |
| `svelte` (the user's installed copy) | Compiler-warning diagnostics (a11y, `state_referenced_locally`, etc.) | Discovered by walking up from the workspace |
| `bun` or `node` | Runs the JS bridge that calls `svelte/compiler` | Found on `PATH`; `bun` preferred for startup speed; override with `SVN_JS_RUNTIME=/path/to/runtime` |

If `bun`/`node` is missing, the run still completes — only the
compiler-warning step is skipped silently.

---

## Quick start

```sh
# Check the current workspace, default human-verbose output
svelte-check-native

# Specific workspace + tsconfig (typical in monorepos)
svelte-check-native --workspace ./apps/web --tsconfig ./apps/web/tsconfig.json

# CI-friendly machine output, exit 1 on any error or warning
svelte-check-native --output machine --fail-on-warnings

# Skip CSS-source warnings (svelte-check default behavior)
svelte-check-native --diagnostic-sources js,svelte

# Reclassify or ignore individual compiler warnings
svelte-check-native --compiler-warnings 'state_referenced_locally:ignore,a11y_no_static_element_interactions:error'
```

---

## How it works

The pipeline transforms each `.svelte` file into a TypeScript wrapper
that tsgo can type-check, runs `svelte/compiler` separately for
non-typecheck diagnostics, then maps everything back to source
positions.

```
                       .svelte source
                            |
            +---------------+---------------+
            |                               |
            v                               v
  +-------------------+         +----------------------+
  |  parse_sections   |         |  svelte/compiler     |
  |  parse_template   |         |  via bun/node bridge |
  |  parse_script     |         |  (warnings only)     |
  +---------+---------+         +-----------+----------+
            |                               |
            v                               |
  +-------------------+                     |
  |  analyze:         |                     |
  |  - find_runes     |                     |
  |  - find_props     |                     |
  |  - find_stores    |                     |
  |  - template_refs  |                     |
  |  - bind_targets   |                     |
  +---------+---------+                     |
            |                               |
            v                               |
  +-------------------+                     |
  |  emit:            |                     |
  |  generates        |                     |
  |  Foo.svelte.ts    |                     |
  |  + line map       |                     |
  +---------+---------+                     |
            |                               |
            v                               |
  +-------------------+                     |
  |  typecheck:       |                     |
  |  overlay tsconfig |                     |
  |  + tsgo subprocess|                     |
  +---------+---------+                     |
            |                               |
            +-----------+-------------------+
                        |
                        v
           +---------------------------+
           |  diagnostic merge +       |
           |  source-map back to       |
           |  .svelte line:col         |
           +-------------+-------------+
                         |
                         v
                   formatted output
                (machine / human / etc.)
```

### Cache layout

```
<workspace>/.svelte-check/
  tsconfig.json            overlay — extends user tsconfig, adds rootDirs,
                           paths-mirror, allowImportingTsExtensions
  tsbuildinfo.json         tsgo's incremental build state
  svelte-shims.d.ts        rune ambients, store unwrap helper, module shims
  svelte/<rel>/Foo.svelte.ts
                           generated TypeScript per .svelte file. Imports
                           are rewritten to `.svelte.ts` so tsgo lands
                           on the overlay file rather than the *.svelte
                           ambient declaration shipped with the svelte
                           package.
```

### Why it's fast

- **Single Rust process.** No per-file Node startup, no svelte2tsx
  subprocess per check.
- **Persistent JS bridge.** One bun/node subprocess imports
  `svelte/compiler` once; every `.svelte` file is sent over stdin to
  the same worker.
- **OXC for JS/TS parsing.** AST construction is roughly 10× faster
  than swc and 50× faster than the typescript parser.
- **Incremental tsgo via tsbuildinfo.** Only changed files are
  re-typed across runs.
- **Parallel discovery.** `walkdir` saturates I/O; per-file parse +
  emit work cleanly distributes across cores.

---

## Benchmarks

A SvelteKit application with ~1200 `.svelte` files. Three scenarios:

- **Cold**: cache cleared, no `tsbuildinfo`, no overlay
- **Warm**: cached run with no source changes
- **Dirty**: one `.svelte` file touched between runs

Wall-clock from `time(1)`, median of 3 runs, all on the same hardware
(M3 Pro, macOS 25.4):

```
                            Cold       Warm       Dirty
                            ----       ----       -----
svelte-check-native          6.32 s     4.95 s     4.56 s
an alternative Rust implementation             19.24 s    10.79 s    10.02 s
svelte-check  --tsgo        13.42 s    12.70 s    13.37 s
svelte-check  (default)     42.89 s    39.36 s    38.89 s
```

The "default" row is what most users run today (`svelte-check`
without the experimental `--tsgo` flag). It's the realistic baseline
to compare against.

```
                       Cold              Warm              Dirty
                     ┌─────┐           ┌─────┐           ┌─────┐
   SCN  6.32 s       │█    │           │█    │           │█    │
                     │     │           │     │           │     │
   SC-rs 19.24 s     │██   │           │█    │           │█    │
                     │█    │           │     │           │     │
   SC --tsgo 13.42 s │██   │           │██   │           │██   │
                     │     │           │█    │           │█    │
                     │     │           │     │           │     │
   SC default 42.89s │█████│           │█████│           │█████│
                     │█████│           │█████│           │█████│
                     │█████│           │█████│           │█████│
                     │█████│           │█████│           │█████│
                     │█████│           │█████│           │█████│
                     │█████│           │█████│           │█████│
                     └─────┘           └─────┘           └─────┘

  scale: 1 row ≈ 5 s wall-clock
```

Speedup factors against the realistic default baseline:

```
                  Cold     Warm     Dirty
svelte-check-native    6.8x     8.0x     8.5x
an alternative Rust implementation        2.2x     3.6x     3.9x
svelte-check --tsgo    3.2x     3.1x     2.9x
```

All four tools report the same diagnostic content for the sources
they cover — the savings come from process / parser / pipeline
architecture, not from skipping checks.

| Tool | Cold | Warm | Dirty | Errors | Warnings (svelte) | Warnings (css) |
|---|---|---|---|---|---|---|
| svelte-check-native | 6.32s | 4.95s | 4.56s | 0 | 44 | (not run) |
| an alternative Rust implementation | 19.24s | 10.79s | 10.02s | 0 | 44 | (not run) |
| svelte-check `--tsgo` | 13.42s | 12.70s | 13.37s | 1¹ | 44 | 4 |
| svelte-check default | 42.89s | 39.36s | 38.89s | 0 | 44 | 4 |

¹ The 1 error in upstream `--tsgo` is a `composite + incremental`
deprecation on its own overlay tsconfig — not user code. We filter
the equivalent in our output as overlay noise.

---

## Feature flags

Run `svelte-check-native --help` for the canonical list. Flags below
that match upstream `svelte-check` behave identically.

### Workspace + config

```
--workspace <path>              Project root to check. Defaults to cwd.
--tsconfig <path>               Path to tsconfig.json (or jsconfig.json).
                                When omitted, walks up looking for one.
--no-tsconfig                   Reserved (errors out today; see Missing flags).
--ignore "dist,build,*.spec.svelte"
                                Comma-separated git-style globs to skip.
                                Matched against workspace-relative paths.
```

### Output

```
--output <fmt>                  human | human-verbose | machine | machine-verbose
                                Defaults to human-verbose. CLAUDECODE=1 forces
                                machine.
--color                         Force ANSI colors (overrides isatty).
--no-color                      Force plain text (overrides isatty).
```

### Diagnostic shaping

```
--threshold <level>             warning (show all) | error (errors only)
--fail-on-warnings              Exit 1 when warnings exist (with no errors).
--diagnostic-sources <list>     Subset of: js, svelte. Default: all supported.
                                Passing "css" exits 2 with a hint — see below.
--compiler-warnings <list>      Reclassify Svelte compiler warnings.
                                Format: code:severity[,code:severity...]
                                Severity: ignore | warning | error.
                                Example:
                                  --compiler-warnings 'state_referenced_locally:ignore,
                                                       a11y_no_static_element_interactions:error'
```

### Debug / introspection

```
--emit-ts                       Print generated TypeScript per file, exit.
--debug-paths                   Print resolved binaries (workspace, tsconfig,
                                tsgo, JS runtime), exit.
--tsgo-version                  Run `tsgo --version`, exit.
--timings                       Print phase-by-phase wall-clock breakdown
                                after the run.
```

### Accepted-but-ignored (upstream-compat shims)

These flags exist in our CLI so scripts written for upstream
`svelte-check` parse without error, but they're no-ops:

```
--incremental                   We always cache.
--tsgo                          We always use tsgo.
--watch                         Use `watchexec`, your editor, or a shell loop.
--preserveWatchOutput           Depends on --watch.
```

---

## What it is NOT

- **Not an IDE extension.** No LSP server, no autocomplete, no hover
  docs, no go-to-definition. Use the official Svelte for VS Code
  extension (or its equivalent in other editors).
- **Not a watch-mode tool.** No `--watch` loop. Pair with `watchexec`
  or your editor's file-watcher.
- **Not a Svelte 4 type checker.** `export let`, `$:`, `<slot>`, `on:`
  are not recognized. For Svelte 4 codebases use upstream `svelte-check`.
- **Not a tsc fallback.** tsgo is the sole TypeScript backend. If tsgo
  isn't installed, the check fails fast with an installation hint.
- **Not a CSS linter.** `--diagnostic-sources css` is explicitly
  rejected (see below).

---

## Missing flags

Flags that exist in upstream `svelte-check` but aren't yet wired:

```
--watch                         Out-of-scope by design. Will not implement.
--preserveWatchOutput           Out-of-scope (depends on --watch).
--no-tsconfig                   Reserved. Errors out today.
--diagnostic-sources css        Hard-rejected (see CSS section).
```

---

## CSS diagnostic source

`svelte-check` runs CSS diagnostics through PostCSS-style linting on
the contents of `<style>` blocks (e.g. vendor-prefix warnings,
unused-selector hints). On the benchmark project this produces about
4 of upstream's 48 warnings.

`svelte-check-native` does NOT ship a CSS linter, and
`--diagnostic-sources css` (or any of `scss`/`sass`/`less`/`postcss`)
exits 2 with:

```
svelte-check-native: --diagnostic-sources "css" requested but CSS linting is
not yet implemented. Drop "css" from the list (or omit --diagnostic-sources
entirely to use the supported defaults: js, svelte).
```

The hard exit is deliberate — silently doing nothing when the user
explicitly asks for CSS coverage is a worse failure mode than telling
them upfront that the support isn't there.

When `--diagnostic-sources` is omitted, the default is `js,svelte`
(NOT `js,svelte,css`), which matches the diagnostic content the tool
actually produces.

---

## Test suite

Two parity bars:

### 1. Upstream svelte-check fixture corpus

The test runner consumes the `.v5` fixture corpus that `svelte-check`
itself ships and uses to verify svelte2tsx output: 63 hand-written
Svelte 5 components covering runes, snippets, props destructuring,
generics, stores, `{#await}`, `{@const}`, `{@attach}`, and edge
cases.

For each fixture we generate the overlay TypeScript, hand it to tsgo,
and compare the diagnostic count to a per-fixture expected value.
Currently 59/63 pass cleanly; the remaining 4 are within their
baseline (see below).

### 2. Local store fixtures

A separate corpus of 24 fixtures specifically exercising Svelte's
store auto-subscribe (`$store`) syntax through the boundaries that
matter for our emit: scoped declarations, destructured stores,
re-exported stores, stores imported from external modules. Locally
authored because upstream's coverage of these patterns is sparse.

### How baselines work

A handful of fixtures from the upstream test corpus are testing
**verbatim emit fidelity** rather than type correctness. They contain
intentionally broken user code (an undefined reference, a generic-
mismatched `$state<T>(0)`, an import from a non-existent module) and
the test passes when our output preserves that user code character-for-
character so that downstream tsgo reports the SAME error a real user
would see.

For these fixtures, `crates/cli/tests/v5_fixtures/baselines.json`
declares an expected `max_errors` count and a `reason`:

```jsonc
{
    "verbatim_emit_fixtures": {
        "runes-best-effort-types.v5": {
            "max_errors": 1,
            "reason": "let { g = foo } = $props() — `foo` is undefined; the fixture
                       preserves the user's reference verbatim. svelte2tsx emits
                       the same error in expectedv2.ts."
        }
        // ...
    }
}
```

A baselined fixture passes if its error count is `≤ max_errors`.
Fixtures NOT in the baseline must produce zero errors. This catches
two regression classes:

- A fixture that should be clean starts producing errors → fails on
  the zero-error rule.
- A baselined fixture starts producing MORE errors than its cap →
  fails on the count rule.

The baseline mechanism is an interim shape. A future change will
replace `max_errors: N` with exact `{code, line, column, message}`
assertions per expected error, so a regression that swaps one error
for a different one (currently silent) gets caught too.

---

## Output formats

All four formats match upstream svelte-check byte-for-byte (modulo
the timestamp prefix). Editor extensions, CI dashboards, and shell
wrappers that consume `svelte-check`'s output work with
`svelte-check-native` unchanged.

### `machine`

```
1776349615385 START "/path/to/workspace"
1776349615386 WARNING "src/lib/X.svelte" 22:5 "..."
1776349615386 ERROR "src/lib/Y.svelte" 8:3 "..."
1776349615387 COMPLETED 1206 FILES 0 ERRORS 44 WARNINGS 15 FILES_WITH_PROBLEMS
```

### `machine-verbose`

```
1776349615385 START "/path/to/workspace"
1776349615386 {"type":"WARNING","filename":"src/lib/X.svelte",
               "start":{"line":21,"character":4},"end":{"line":21,"character":13},
               "message":"...","code":"state_referenced_locally",
               "codeDescription":{"href":"https://svelte.dev/docs/svelte/compiler-warnings#state_referenced_locally"},
               "source":"svelte"}
1776349615387 COMPLETED 1206 FILES 0 ERRORS 44 WARNINGS 15 FILES_WITH_PROBLEMS
```

### `human` / `human-verbose`

`human` is the colored compact form (file:line:col + Error/Warn label
+ message). `human-verbose` (default) adds a banner prelude and a
3-line code frame around each diagnostic with caret underlines, in
cyan.

---

## CI/CD usage

Recommended invocation in CI:

```sh
svelte-check-native \
  --workspace . \
  --tsconfig ./tsconfig.json \
  --output machine \
  --threshold warning \
  --fail-on-warnings
```

Exit codes:

- `0` — no errors (and no warnings if `--fail-on-warnings`)
- `1` — errors detected (or warnings with `--fail-on-warnings`)
- `2` — invocation error (bad flag, missing tsconfig, missing tsgo)

CI scripts can grep for `^.* COMPLETED ` to extract the summary line
without parsing every diagnostic, or pipe `--output machine-verbose`
into `jq` for full structured access.

The compiler-warning bridge runs even when `bun`/`node` isn't
installed (silently no-ops). To force it OFF, pass
`--diagnostic-sources js`.

---

## AI-agent usage

Three properties matter for agent loops:

- **Sub-5-second feedback** on a typical project. An agent that makes
  10 changes and re-checks each iteration spends ~50s with
  `svelte-check-native` vs ~390s with the default upstream
  `svelte-check` — that's 6 minutes of wait per agent session reduced
  to under one.
- **Stable JSON output** (`machine-verbose`) with `code`,
  `codeDescription.href`, `start`, `end`, `severity`. Same shape
  upstream emits, so agents can route on `code` slugs.
- **`CLAUDECODE=1`** auto-switches the default format to `machine`,
  matching what upstream does, so prompts that say "run
  svelte-check-native" don't need to spell out `--output`.

The compiler-warning bridge also gives agents access to documented
URLs (`codeDescription.href`) for every warning — useful for
explaining issues without context-window-expensive lookups.

---

## License

MIT
