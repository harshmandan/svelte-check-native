# Changelog

All notable changes to `svelte-check-native` will be documented in this
file. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [SemVer](https://semver.org/spec/v2.0.0.html).

## [0.1.2]

### Fixed

- **`$state<Promise<T>>(new Promise(() => {}))` no longer fires a
  spurious TS2769 "Promise<unknown> not assignable" diagnostic.** The
  `$state` shim now has two overloads (normal `T` + no-arg) instead of
  four; the `null` / `undefined` literal-type overloads collided with
  TypeScript's overload resolution on `$state<Promise<T>>(...)`, where
  an explicit `<T>` no longer propagates as contextual type to the
  argument when the overload set includes literal-type variants. The
  inner `new Promise(() => {})` then widens to `Promise<unknown>` and
  no overload matches. This is TypeScript behavior across tsc and
  tsgo. To keep the bind:this pattern the dropped overloads were
  protecting, the emit layer now rewrites
  `let X: Type = $state(null | undefined)` to
  `let X: Type = $state<Type>(null | undefined)` — explicit generic
  from the annotation replaces the literal-type overloads. Net:
  real-world parity bench picks up 2 errors on the reference
  SvelteKit project (which just landed a `$state<Promise<T>>(…)`
  trendline refactor) and 2 errors on `inference-playground`.

## [0.1.1] — docs update

## [0.1.0] — first public release

First publishable cut: drop-in CLI replacement for upstream `svelte-check`,
restricted to Svelte 5+ syntax, powered by `tsgo` for TypeScript
diagnostics and the user's own `svelte/compiler` install for compiler
warnings.

### Features

- **CLI surface parity** with upstream `svelte-check` for the flags this
  project supports: `--workspace`, `--tsconfig`, `--output`,
  `--threshold`, `--fail-on-warnings`, `--diagnostic-sources`,
  `--compiler-warnings`, `--ignore`, `--color` / `--no-color`. Same
  exit codes (`0` clean, `1` problems, `2` invocation error).
- **All four output formats** — `human`, `human-verbose`, `machine`,
  `machine-verbose` — byte-equivalent to upstream so existing editor
  integrations / CI parsers / shell wrappers work without modification.
- **TypeScript diagnostics via `tsgo`** (`@typescript/native-preview`).
  No bundled tsgo — discovered in the user's `node_modules` chain.
  Override with `TSGO_BIN=/path/to/tsgo`.
- **Compiler warnings via the user's `svelte/compiler`** through a
  multi-worker `bun` / `node` bridge subprocess pool. Auto-detects the
  workspace's installed `svelte` package; bridge silently no-ops when
  no JS runtime is on `PATH`.
- **Per-region source maps** that translate every overlay diagnostic
  back to the precise `.svelte:line:column` it came from.
- **Real-world parity verified** against a heavy SvelteKit + TypeScript
  app (1206 `.svelte` files): byte-equivalent diagnostic content to
  upstream `svelte-check` with CSS disabled on both
  (`--diagnostic-sources 'js,svelte'` upstream, `'ts,svelte'` ours —
  semantically identical) → `0 errors / 44 warnings / 15 files with
  issues` from both tools.
- **`--tsgo-version`** — print resolved tsgo binary path + its
  `--version` output, for verifying `@typescript/native-preview` is
  at the expected version.
- **`--tsgo-diagnostics`** — forward `--extendedDiagnostics` to tsgo
  and print its perf/memory stats block (file/line/symbol counts,
  memory, phase timings) after the run. Same intent as upstream
  `svelte-check-rs`'s flag of the same name.
- **`--tsgo`** — accepted as a no-op for command-line compatibility
  with upstream `svelte-check` (tsgo is always on in our pipeline).
- **Partial Svelte 4 syntax compat** — `export let foo` and
  `export { name as alias }` specifier form are lifted into the
  synthesized `Props` type. Full Svelte 4 support (`<slot>`,
  `on:event`, `$:` reactive statements) lands in v0.2.
- **Coding-agent CLI auto-detection**: output defaults to `machine`
  when `CLAUDECODE=1`, `GEMINI_CLI=1`, or `CODEX_CI=1` is set.

### Performance

On the same ~1200-file workload (M-series 8-core, warm cache, median of
3 runs):

| Tool                      | Warm     |
| ------------------------- | -------- |
| `svelte-check-native`     | **~3 s** |
| `svelte-check-rs`         | ~11 s    |
| `svelte-check --tsgo`     | ~13 s    |
| `svelte-check` (default)  | ~40 s    |

Cold (no cache, fresh `bun` import): ~7–8 s.

The speed comes from a multi-worker `svelte/compiler` bridge (`cores/2`
parallel `bun` / `node` subprocesses each importing `svelte/compiler`
once), `rayon`-parallel parse + analyze + emit, OXC for JS/TS parsing,
and tsgo's incremental `tsbuildinfo` reused across runs.

### Distribution

- **npm package** with platform-specific binaries: `svelte-check-native`
  (the wrapper) plus `svelte-check-native-{darwin-arm64, darwin-x64,
  linux-arm64, linux-x64, win32-x64}` (the binaries). npm picks the
  matching platform package automatically via `optionalDependencies` +
  `os` / `cpu` fields.
- **Cross-built locally** via `cargo-zigbuild` (Zig as the C linker)
  from a single macOS host.

### Cache layout

Generated overlay TypeScript and tsgo's incremental build state live at:

- `<workspace>/node_modules/.cache/svelte-check-native/` (default —
  gitignored everywhere by the standard `node_modules/` ignore rule),
  with
- `<workspace>/.svelte-check/` as the fallback for fresh-clone or
  no-deps fixtures.

### Out of scope (will not implement)

- Watch mode (`--watch`, `--preserveWatchOutput`) — pair with
  `watchexec` or your editor.
- LSP server, autocomplete, hover, go-to-definition — use the official
  Svelte for VS Code extension.
- Svelte 4 syntax (`export let`, `$:`, `<slot>`, `on:` directives).
- `tsc` fallback — tsgo is the only TypeScript backend.
- CSS linting — `--diagnostic-sources css` is hard-rejected with a hint.
