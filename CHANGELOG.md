# Changelog

All notable changes to `svelte-check-native` will be documented in this
file. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [SemVer](https://semver.org/spec/v2.0.0.html).

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
  app (~1200 `.svelte` files): same diagnostic content as upstream
  `svelte-check` with the same flags (`0 errors / 10 warnings / 7 files`
  on that workload).
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
