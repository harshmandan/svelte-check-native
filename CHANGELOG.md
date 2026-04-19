# Changelog

All notable changes to `svelte-check-native` will be documented in this
file. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [SemVer](https://semver.org/spec/v2.0.0.html).

## [0.3.0]

### Added — component-prop + event typing

- **Phase 5 `satisfies ComponentProps<typeof Comp>` trailer.**
  Every `<Component ... />` in a template emits a `props: {...}
  satisfies InstanceType<typeof __svn_C_N>['$$prop_def']` in the
  overlay. Missing required props fire TS2741 at the user's
  `<Component>` opening tag position (new `TokenMapEntry`
  byte-span source map surfaces the diagnostic at the user-visible
  site instead of dropping it in the synthesized region). Closes
  the reverted `af30c56` / `4f4e613` cycle from v0.2.5.
- **Typed event-handler narrowing via `$$Events`.** When a child
  component declares `interface $$Events` or `type $$Events`, the
  consumer's `<Child on:myevent={handler}>` narrows `handler`'s
  parameter to `(e: CustomEvent<E[K]>) => any`. Wrong-payload
  handlers fire TS2345 at the `on:event={handler}` directive
  position.
- **DOM `bind:` coverage expanded.** In addition to the v0.2
  `bind:contentRect` / `bind:contentBoxSize` / etc. family:
  - HTMLImageElement: `naturalWidth`, `naturalHeight`.
  - HTMLMediaElement: `duration`, `seeking`, `ended`, `readyState`.
  - HTMLElement layout: `clientWidth`, `clientHeight`,
    `offsetWidth`, `offsetHeight` (emitted inline at the
    bind-site so block-scoped iterator names in
    `bind:clientWidth={items[i].width}` resolve correctly).
  - Bidirectional fixed-type: `bind:checked` (boolean),
    `bind:files` (FileList | null).
  - Bidirectional attribute-aware: `bind:value` on `<input
    type="number">` / `<input type="range">` → number; on
    `<input>` default / `<input type="text">` / `<textarea>` →
    string.
- **`bind:this` on DOM elements.** Both simple-identifier and
  member-expression targets (`bind:this={refs.input}`) get a
  type-check emitted inline at the bind-site. Matches upstream's
  assignment-direction semantics.
- **`bind:X={getter, setter}` get/set form.** Svelte 5's two-
  function bind syntax modeled as `PropShape::GetSetBinding`
  emitting `name: (getter)()` so phase-5 `satisfies` sees the
  required prop present.
- **Byte-span source maps.** New `TokenMapEntry` alongside
  `LineMapEntry` in the typecheck mapper. Diagnostics in
  synthesized regions get remapped to tightest-matching source
  byte spans; drops only when neither token-map nor line-map
  covers. Per-feature post-scans (`collect_satisfies_token_map`,
  `collect_on_event_token_map`, `collect_dom_binding_token_map`,
  `collect_bind_this_check_token_map`) pair overlay sites 1:1
  with user source.

### Added — bench tooling

- **`scripts/bench.mjs --mode parity`.** Runs our binary +
  upstream `svelte-check` + `svelte-check --tsgo` (when the
  workspace's svelte-check version supports it) on the same
  workspace; prints a side-by-side comparison of FILES, ERRORS,
  WARNINGS, FILES_WITH_PROBLEMS counts. Exits non-zero on drift
  from the best-available upstream baseline. Solution-style
  tsconfig redirects are detected + propagated so upstream runs
  from the same effective workspace our binary hops to.
- **Goal, per NEXT.md:** diagnostic counts match upstream
  `svelte-check` (preferred) or at minimum `svelte-check
  --tsgo`. Current status: control-svelte-4 matches `svelte-check
  --tsgo` exactly (1124 FILES, 1 ERROR, 2 WARNINGS, 3
  FILES_WITH_PROBLEMS).

### Fixed — pre-existing test failures

- **SvelteKit kit-file diagnostics surface correctly.** Two bugs
  that silently dropped diagnostics on `+page.ts` / `+layout.ts`
  / `+server.ts` after `kit_inject` spliced `: T` annotations:
  - `CacheLayout::original_from_generated` was stripping `.ts`
    from kit mirror paths (`+page.ts` → `+page`), so tsgo's
    reported path failed reverse-mapping and the diagnostic
    never reached the user.
  - Kit overlays carry empty `line_map`/`token_map`; the
    position translator previously dropped any diagnostic
    without a map entry. `MapData` now carries an
    `identity_map: bool` flag (set for kit inputs) so positions
    pass through unchanged — correct because `kit_inject`
    splices only same-line `: T` annotations and never adds
    lines.
- **Hoisted-import columns match source.** `split_imports` now
  preserves each hoist span's leading same-line whitespace, so
  overlay imports keep the source indentation. Fixes column
  drift on TS2307 module-resolution errors: `import nope from
  '../../outside'` reports col 21 (matching upstream's expected)
  instead of col 17 (overlay with stripped indent).
- **Hoisted-type stubs use richer type annotation.**
  `script_split`'s `declare const <body-local>` stub emitted for
  body-local names referenced by hoisted types now uses
  `{ [key: string]: any } & ((...args: any[]) => any)` instead
  of plain `any`. Preserves `keyof typeof <local> = string`
  (previously widened to `string | number | symbol`, tripping
  TS1023 on user `<local>[stringKey]` subscripts) and callable
  references (`typeof fn`). Closes two pre-existing test
  failures.
- **Doctest fence.** Illustrative TypeScript code in
  `emit_component_call`'s doc comment now lives in a
  ```` ```text ```` fenced block; rustdoc no longer tries to
  compile it as Rust.

### Fixed — corner cases

- **`bind:NAME` shorthand on components.** Bare-shorthand form
  (`<Child bind:items />` desugaring to `<Child
  bind:items={items}>`) now emits as `PropShape::Shorthand`
  rather than being silently dropped. Closed 36 false positives
  on the initial phase-5 `satisfies` bench pass where
  consumers of v0.2.5 bind-supporting components were reported
  as missing the bound prop.
- **`bind:X={getter, setter}` consumer-side.** Svelte 5's
  two-function bind form is now modeled; consumers of children
  with required props that the user passes via get/set no
  longer fire spurious "missing required prop" TS2741.

### Known limitations (match upstream behavior)

These are intentional gaps where upstream `svelte-check` is
deliberately permissive; our tool matches that laxity to
preserve drop-in parity. Users who want stricter checks should
file a request for an opt-in flag.

- **`bind:value` on `<select>`.** Accepts any type (upstream
  types `HTMLSelectAttributes['value']` as `any`). Would need
  `<option>` value-type union inference to be stricter.
- **`bind:group`** on `<input type="checkbox|radio">`. Silent;
  no target-type check. Upstream's Binding.ts:99-108 emits
  `EXPR = __sveltets_2_any(null);` (widen-to-any). Our
  skip-entirely produces the same observable no-error
  behavior.
- **Cross-HTML*Element distinctions for `bind:this`.** TS's DOM
  lib treats HTML element subtypes as structurally compatible
  (HTMLDivElement and HTMLInputElement share all of HTMLElement
  + HTMLDivElement's only unique field `align` is optional).
  `<input bind:this={divRef}>` where `divRef: HTMLDivElement`
  accepts silently. Matches upstream; not fixable without
  inventing our own DOM hierarchy.
- **Simple-identifier `bind:` target checks.** For `let num:
  number = 0; <input type="number" bind:value={num}>`, the
  pre-existing definite-assign rewrite (`num = undefined as
  any;`) widens `num`'s flow type to `any` before the bind
  contract check runs inside `__svn_tpl_check`, so wrong-typed
  targets don't fire on simple identifiers. Member-expression
  targets (`state.num`) skip the rewrite and DO get checked.

### Known bench discrepancies

- **Upstream default `svelte-check` reports ~5× more FILES** on
  large workspaces (control-svelte-4: 6511 FILES vs ours 1124
  vs `svelte-check --tsgo` 1125). Upstream default crawls all
  `.svelte/.ts/.js/.d.ts` on disk, including declaration files
  that aren't part of the type-check surface. Our FILES matches
  `svelte-check --tsgo` (the meaningful "what was actually
  type-checked" count). Users who need default-svelte-check
  denominator parity should benchmark against `--tsgo`.

### Performance

- control-svelte-4 bench (1124 files): warm 2.31s median (v0.2.5
  baseline: 2.30s, within noise). Cold 3.4s, dirty 2.4s.

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
