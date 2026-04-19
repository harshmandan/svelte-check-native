# Changelog

All notable changes to `svelte-check-native` will be documented in this
file. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [SemVer](https://semver.org/spec/v2.0.0.html).

## [0.2.5]

### Fixed — monorepo parity

- **Monorepo-root parity closed.** Running `svelte-check-native
  --workspace .` at the root of a TS project-references solution
  (`tsconfig.json` with `"files": []` + `"references": [...]` — the
  common SvelteKit-monorepo root shape) no longer misreports
  thousands of `Cannot find module '$lib/...'` errors. The CLI
  detects solution-style tsconfigs at resolve time and redirects
  the tsconfig and workspace to a sub-project's `tsconfig.json`
  that carries real `compilerOptions.paths`. Prints a diagnostic
  line on stderr explaining the redirect. Root-workspace runs now
  produce the same diagnostics as the app-scoped run
  (`cd src/apps/foo && svelte-check-native --workspace .`) on the
  reference SvelteKit monorepo — 0 user errors. Pass
  `--tsconfig <path>` to override the heuristic.
- **Diagnostic-mapper drop.** Overlay diagnostics whose position
  falls outside every `LineMapEntry` (synthesized scaffolding —
  component-call blocks, default-export type, template wrapper,
  void block) are now dropped rather than clamped to the nearest
  verbatim source line. Previously surfaced as FPs at positions
  the user's code doesn't occupy. Mirrors upstream svelte-check's
  source-map-driven filter. Biggest practical win: real-world
  codebases using bits-ui / shadcn / tailwind-variants no longer
  report "union type too complex" / "Omit'd prop doesn't exist"
  at synthesized component-call sites.

### Added — emit fidelity

- **Spread props into component literal** (#41 phase 1).
  `<Comp {...rest}>` emits `{...(rest)}` inside the props object
  so TS checks spread against the target Props.
- **Implicit children synthesis** (#41 phase 2). Component with
  non-snippet body content emits a synthesized
  `children: () => __svn_snippet_return()` key so Svelte-5
  components declaring `children: Snippet` (required) accept
  `<Comp>body</Comp>` cleanly.
- **`bind:this` on components** (#41 phase 3) emits
  `x = __svn_inst_N;` after construction. x's declared type now
  gets checked against the component's instance shape; previous
  `!` definite-assign rewrite handled the declaration side but
  missed the type-compatibility signal.
- **Svelte-4 with `<slot>`** (#41 phase 4) wraps default-export
  Props in `Partial<>` — mirrors upstream's
  `__sveltets_2_isomorphic_component_slots` vs plain variant.
- **Component `bind:NAME`** emits as a regular prop key (e.g.
  `bind:value={x}` → `value: x`), so child Props declarations
  catch type mismatches.
- **DOM one-way-not-on-element bindings** (`bind:contentRect`,
  `bind:contentBoxSize`, `bind:borderBoxSize`,
  `bind:devicePixelContentBoxSize`, `bind:buffered`,
  `bind:played`, `bind:seekable`) emit phantom
  `__svn_any_as<TYPE>(expr)` contract checks against the declared
  target type.

### Added — SvelteKit coverage

- `$types` injection for `+server.ts` handler signatures.
- `kit_inject` covers the full `+page.ts` / `+layout.ts` /
  `+page.server.ts` / `+layout.server.ts` family: `ssr` / `csr` /
  `prerender` / `trailingSlash` get their fixed-union types;
  `load`'s first parameter gets
  `{Page|Layout}{Server?}LoadEvent`.
- Import-following via overlay-for-every-Svelte. User code
  importing `type { Foo } from './Panel.svelte'` resolves to the
  overlay's hoisted type rather than getting silently erased.

### Added — tests + packaging

- **Full upstream sample corpus**. 22 previously-skipped
  htmlx2jsx samples unskipped (stale pre-v0.2 skip list); all 240
  `.svelte`-bearing svelte2tsx samples now run (was 57 `.v5`-only).
  387 emit snapshots total, pure emit-shape regression coverage.
- **Exact-shape baselines for v5 / v5-stores fixtures**.
  `baselines.json` now carries per-error
  `{code, line, column, message_contains?}` lists;
  `CAPTURE_BASELINES=1` regenerates on deliberate emit changes.
- **npm dist generator-driven** via `scripts/prepare-release.mjs`
  into a gitignored `dist-packs/pkgs/`; the repo no longer
  commits per-platform package directories.

## [0.2.0] — Svelte 4 + Svelte 5 parity

### Added

- **Svelte 4 surface support**: `export let`, `$:` reactive
  declarations + statements, `on:event` directives,
  `<slot>` / named slots with `let:` destructuring,
  `createEventDispatcher`, `bind:` on components,
  `bind:this={x}` on elements, renamed exports
  (`export { name as alias }`), `$$Props` / `$$Events` /
  `$$Slots` interfaces, `$$slots` / `$$props` / `$$restProps`
  ambients, `export function` / `export const`. Every
  Svelte-4-specific helper lives under
  `crates/*/src/svelte4/` with a `// SVELTE-4-COMPAT` marker so
  removal is mechanical when Svelte 4 is officially retired.
- **Parity gate**: 1000-file mid-migration SvelteKit workspace
  type-checks at 0 real errors, tying upstream
  `svelte-check --tsgo`.

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
