# Working conventions for Claude Code / AI-assisted contributions

This file is loaded into every Claude Code session in this repo. Read
`README.md` for the public-facing overview, and `ROADMAP.md` (local,
gitignored) for the current scoreboard, what's next, and documented
limitations. This file is the shorter "rules of engagement" layer on
top of both.

## Project at a glance

- **Goal:** a CLI-only type checker for Svelte projects, written in
  Rust, powered by tsgo. Drop-in replacement for upstream `svelte-check`
  on the CLI surface (same flags, same output formats, same exit codes,
  same `<N> FILES` denominator).
- **Svelte 4 and Svelte 5 are both supported.** Svelte-4 surface
  features (`export let`, `$:`, `on:event`, `<slot>` / named slots,
  `createEventDispatcher`, `bind:` on components, renamed exports)
  all shipped in the v0.2 parity push. Parity gate: a 1000-file
  mid-migration SvelteKit workspace type-checks clean, tying upstream
  `svelte-check --tsgo` at 0 real errors.
- **No bundled tsgo.** We discover the user's `@typescript/native-preview`
  install in `node_modules`, preferring the platform-native binary over
  the JS wrapper. `TSGO_BIN` env var is the override.
- **Upstream submodule:** `language-tools/` is a pinned submodule of
  `sveltejs/language-tools` — used as the source of truth for upstream's
  CLI behavior, the 63 `.v5` test fixtures from `svelte2tsx` that form
  our parity gate, and the `isKitFile` / `findFiles` algorithms whose
  output we mirror byte-for-byte.

## Scope discipline (repeated here because it's easy to forget)

Out of scope — do NOT implement:

- LSP server / editor integration
- Autocomplete, hover, go-to-definition, rename, code actions
- Watch mode (use `watchexec` externally)
- tsc fallback (tsgo only)
- Formatting
- CSS lint rules beyond the narrow vendor-prefix carve-out in ROADMAP

Svelte-4 compat is shipped but isolated: every Svelte-4-specific
helper goes under `crates/*/src/svelte4/` with a `// SVELTE-4-COMPAT`
marker at each callsite. When Svelte 4 is officially retired the
removal is mechanical — delete the submodule and grep for the marker.

### Svelte-4 feature coverage (internal reference)

What each Svelte-4-specific surface feature maps to in the emit:

| Syntax                                 | Handled by                                  |
| :------------------------------------- | :------------------------------------------ |
| `export let foo` / `export let foo: T` | `crates/analyze/src/props.rs` — synthesized Props entry |
| `export let foo = v` (untyped default) | widen to `any` in script_split so Props stays permissive |
| `export { name as alias }`             | specifier-form in `crates/analyze/src/props.rs` |
| `$: NAME = EXPR` (declaration)         | `crates/emit/src/svelte4/reactive.rs` — `let NAME = __svn_invalidate(() => EXPR); void NAME;` |
| `$: EXPR;` (statement)                 | arrow-wrap: `;() => { $: EXPR };` so body's TDZ is function-scope not top-level |
| `$: ({a, b} = expr)` (destructure)     | rewrite to `let {a, b} = __svn_invalidate(() => (expr));` |
| `on:event={handler}` on components     | `ComponentInstantiation.on_events` + emit `$inst.$on("evt", handler);` after construction |
| `on:event={handler}` on DOM elements   | translated to `onXXX` attribute key in `svelteHTML.createElement` call |
| `<slot>` / `<slot name="X">` / `<slot {prop}>` | synthesized `children` / named Snippet props on the generated Props type |
| `<Foo let:item>` slot-let subtree      | let-binding scope opened before `emit_component_call`; `let item: any; void item;` |
| `createEventDispatcher<T>()`           | dispatcher call recognized; `$on` signature keeps `handler: (...args: any[]) => any` (matches upstream's `__sveltets_2_with_any_event` widening of the default export) |
| `bind:` on components                  | silently dropped at analyze; `__SvnPropsPartial<P>` makes the target prop optional so nothing fires |
| `bind:this={x}` on elements            | `BindThisTarget` recorded; script's `let x: T` gets the `!` definite-assign rewrite so async bind doesn't trip TDZ |
| `$$Props` / `$$Events` / `$$Slots`     | `crates/analyze/src/svelte4/` — pulls the interface as the props-type source if present |
| `$$slots` / `$$props` / `$$restProps` ambients | injected per-file in the script prelude so references resolve |
| `export function` / `export const`     | surfaced on the synthesized instance return; consumers see the field via `__SvnInstance<P>` + default-export type |

Parity gate: 0 errors on a 1000-file mid-migration SvelteKit
monorepo, tying upstream `svelte-check --tsgo`. See `ROADMAP.md`
for the exact bench numbers and known tsgo ceiling limits.

In scope: CLI flags matching upstream, byte-identical output formats,
tsgo invocation, diagnostics mapping back to `.svelte` source,
`<N> FILES` denominator matching upstream's `|entries ∪ diagnostic
files|` semantic.

## Commit-and-continue

- **Commit after every meaningful local step,** even if code is broken or
  tests are red. Commits are restore points, not polished artifacts.
- **Never `git push` without explicit user confirmation** each time.
  Session-level approval does not carry over to future sessions or
  branches.
- Prefer small, frequent commits over large "clean" ones. A half-working
  snapshot is always more valuable than no snapshot.
- Commit message style: imperative mood, lowercase, one concise line.
  Body optional; include when the "why" isn't obvious from the diff.

## Style & quality bar

- **Rust edition 2024.** `rust-version = "1.85"` in every crate's
  Cargo.toml (inherited from workspace).
- `cargo fmt` clean. `cargo clippy --workspace --all-targets -- -D warnings`
  clean. `cargo test` — the scoreboard count must be monotonically
  non-decreasing per commit.
- No `unwrap()` / `expect()` in library code except with a clear
  invariant comment. Binary entry points (`main.rs`) may use
  `anyhow::Result` and propagate. Test code may use both freely (it's
  supposed to panic loudly on unexpected states).
- No `TODO:` / `FIXME:` comments checked in without a tracking task
  somewhere (issue, PR description, or local `ROADMAP.md`). Scratch
  TODOs belong in a working branch, not main.

## Architecture rules

1. **No character-level scanners for embedded JS/TS.** Use `oxc_parser`
   and walk the AST. Hand-rolled destructuring/expression scanners are
   fragile by construction; an AST-level pattern match makes whole
   classes of bug categorically impossible.
2. **Two-phase transformer.** Phase 1 (analyze) populates a
   `SemanticModel` including a `VoidRefRegistry`. Phase 2 (emit) reads
   from the model. Never mutate the model during emit. Never register
   new names during emit.
3. **Single source of truth for synthesized-name registry.** Every name
   the emit crate creates (template-check wrapper, action attrs, bind
   pairs, store aliases, prop locals) is registered once and emitted
   in one consolidated `void (...)` block. No per-feature `void <name>;`
   sprinkling.
4. **One canonical `TsConfig` struct.** In `crates/core/`. Used by both
   CLI config resolution and the overlay builder. No parallel
   JSON-reading shortcuts.
5. **Pre-allocated buffers.** Estimate output size from AST, allocate
   `Vec<u8>::with_capacity(n)` once. Use `write!` macro, not `format!`
   + `push_str`.
6. **Synthesized-name prefix:** `__svn_*`. Used for every name the emit
   crate creates so they're trivially distinguishable from user code in
   diagnostics.
7. **Component instantiations emit as `new $$_CN({target, props})`
   through the `__svn_ensure_component` wrapper.** Each `<Comp ...>`
   in the template emits as:

   ```ts
   { const __svn_CN = __svn_ensure_component(Comp);
     const __svn_inst_N = new __svn_CN({ target: __svn_any(), props: { ... } });
     __svn_inst_N.$on("click", handler);  // one per on:event directive
   }
   ```

   The wrapper handles both our callable-default overlays and
   third-party Svelte-4-style classes uniformly. The intermediate
   `const __svn_CN = ...` local is load-bearing — it's what lets TS
   bind generic components' `<T>` at the `new` site against concrete
   prop values (dropped local → `T` resolves to `unknown`, snippet
   arrows fire implicit-any).

   Constructor's `props?` slot is `__SvnPropsPartial<Props>` =
   `{[K in keyof P]?: P[K] | null}` — required props stay optional at
   the call site (bind:directives, spreads, implicit `children` from
   the template body don't show up in the emitted object literal) and
   `bind:this`-style `T | null` patterns pass through without TS2322
   nullability mismatch. The `on${string}` index-signature intersection
   that used to live here was removed when `on:event` directives
   switched to the `$inst.$on("evt", fn)` shape — without it, user
   props whose name happens to start with `on` (e.g. `oneTouchReaction`)
   no longer collide with the widen's callable-or-primitive value
   union.

   The instance-local `__svn_inst_N` is only emitted when the
   component has at least one `on:event` directive to dispatch —
   otherwise the `new $$_CN(...)` statement stands alone. `$on`'s
   signature (`handler: (...args: any[]) => any`) gives the arrow its
   contextual-callable typing, so `({detail}) => …` destructures
   without implicit-any.

   Overlay default exports are typed `import('svelte').Component<Props>`
   — the function form. Matches `ComponentProps<typeof Foo>`'s
   built-in constraint directly and keeps generics expressible.

8. **New emit shapes are tsgo-validated on a hand-written fixture
   before implementation.** Any change to what the emit crate produces
   — new helper, new component-call shape, new binding pattern — is
   first expressed as hand-written TS in a throwaway fixture and
   compiled with tsgo. The validation must prove (a) the clean case
   produces zero diagnostics and (b) a deliberately-broken companion
   file produces exactly the expected diagnostics with the expected TS
   codes at the expected positions. Only after the fixture gates green
   does implementation in Rust begin. `design/phase_a/` is the
   reference for this discipline; the validator for the callable-shape
   emit landed as part of Phase A's deliverable.

## Testing discipline

Our binary has three internal stages — `parse → emit → tsgo → map`.
The test strategy mirrors those stages. Each layer is tested
independently so a red signal points at exactly one stage.

**Stage 1 — emit shape (`emit_snapshots`).** Primary gate. Per-sample
`expected.emit.ts` snapshots locked against our binary's `--emit-ts`
output. No tsgo in the loop. Mirrors upstream svelte2tsx's
`expectedv2.js` pattern against *our* emit. `UPDATE_SNAPSHOTS=1
cargo test --test emit_snapshots` accepts deliberate emit changes;
default mode fails on any mismatch with a contextual diff. ~190
snapshots across three corpora:

  - `svelte2tsx_v5/` — upstream's 63 `.v5` samples (full-component).
  - `htmlx2jsx/` — upstream's ~125 template-control-flow samples,
    filtered against a 22-sample Svelte-4 skip list.
  - `bugs/` — our grey-box fixtures.

  Runs in <1 s. Any emit change that's not deliberate must fail this
  gate before anything else is considered.

**Stage 2 — tsgo is trusted.** No tests; it's the TypeScript team's
code. Integration tests below cover "does the emit work end-to-end"
without pretending to test tsgo itself.

**Stage 3 — error mapping (unit tests).** `crates/typecheck/src/lib.rs`'s
test module exercises `map_diagnostic` in isolation — line-map
translation, path reverse, edge cases (empty map, gaps, synthesized
lines). 42 unit tests, no subprocess, no samples.

**Integration — targeted, small (`bug_fixtures`, `v5_fixtures`,
`v5_stores_fixtures`).** Self-contained fixtures that do go through
the whole pipeline including tsgo. Each asserts either zero errors
or an exact expected-errors list. These catch "emit-plus-tsgo
interaction" bugs — the kind where emit looks fine and tsgo looks
fine but the combination has a surprise. Kept small on purpose;
broad type-check surveying is the emit_snapshots job, not these.

**End-to-end — `upstream_sanity`.** Reuses upstream's
`test-sanity.js` unmodified via a node shim. Submodule bump =
upstream test update applied for free. Known-failing today on
SvelteKit-ambient-typing cases (scoped out for v0.1).

**Discovery (not tests).** Real-world repos in `bench/` are *not*
part of `cargo test`. They're used interactively to find bug classes
that get extracted into new `bug_fixtures/<NN>-*` entries and locked
by the suites above. Their error counts are not a shipping metric.

**Bench targets for perf measurement (`scripts/bench.mjs`):**
- `bench/control-svelte-4` — the 1000-file parity-gate target.
  Mid-migration SvelteKit monorepo, mostly Svelte-4-syntax
  components. The "1000-file mid-migration" number in the public
  README and CHANGELOG refers to this workspace's sub-app subset
  (`src/apps/sub-app`, ~1124 files after monorepo-root auto-
  escape). Ties upstream `svelte-check --tsgo` at 0 user errors.
- `bench/control-svelte-5` — the latest fresh extract from the
  same upstream repo's `main` branch (Svelte 5.55+ and further
  along the Svelte-5 migration). Used to spot regressions as the
  codebase moves forward.

The committed bench script (`scripts/bench.mjs`) takes
`--target <path>` or `$BENCH_TARGET` — no project name hardcoded
so the scenario is reproducible against any workspace.

- **Spec-first.** Write the test (snapshot or fixture) before the
  implementation. Snapshot workflow: add `input.svelte`, run
  `UPDATE_SNAPSHOTS=1 cargo test --test emit_snapshots` once the
  emit is right, review `git diff`, commit.
- **`cargo test` is the scoreboard.** `emit_snapshots` count and
  `bug_fixtures` count both show in `README.md`.

## Diagnostic method — "diff the real upstream artifact"

When a diagnostic fires on our binary that upstream doesn't (or vice
versa), the debugging path that SHORT-CIRCUITS hours of speculation:

1. **Anchor on a real failing file.** Pick the exact `.svelte` file
   from a bench or submodule test fixture where the diagnostics
   diverge. Don't build a synthetic reduction first — the real file
   captures all the context that matters (`$state()` uninit patterns,
   `bind:` combinations, destructured `$props()`, slot shapes, etc.).

2. **Read upstream's actual overlay for that file.** Install
   `svelte2tsx` from any bench's `node_modules` (different versions
   exist across benches — any recent one is fine) and run it directly:

   ```sh
   node -e "
     import('<bench>/node_modules/.../svelte2tsx/index.mjs').then(m => {
       const src = require('fs').readFileSync('/path/to/File.svelte', 'utf8');
       console.log(m.svelte2tsx(src, { filename: 'File.svelte', isTsFile: true }).code);
     });
   "
   ```

3. **Read our overlay for the same file.** Run our binary with
   `--emit-ts` on the same directory:

   ```sh
   target/debug/svelte-check-native --workspace <dir> --tsconfig <cfg> --emit-ts
   ```

4. **Side-by-side diff at the diagnostic site.** Focus on the
   specific LINES tsgo flags (grep the overlay for the relevant
   variable names, not whole-file diffs). The structural delta is
   usually a wrapper / lambda / parenthesization / extra intersection
   — not a primitive type difference.

5. **Lock upstream's shape as a tsgo fixture first** (Rule #8 of
   "Architecture rules"). Minimal `.ts` files under `design/<topic>/`
   that produce exactly the expected diagnostics from tsgo. Commit
   before any Rust change. If the fixture doesn't reproduce the
   desired behavior, the theory is wrong and coding won't fix it.

6. **Only then port the shape into our emit.**

### When to deploy it

- Any time `cargo test --test upstream_sanity` or a bench's error
  count diverges from `svelte-check --tsgo` on the same workspace.
- Any time a theory about "why TS should fire here" conflicts with
  what `svelte-check --tsgo` actually reports.
- Any time you catch yourself writing "TS should do X here" — the
  real artifact is the ground truth; reading it is almost always
  faster than reasoning about it.

### Case study (2026-04-20)

`control-svelte-4` FigmaPopup FP: we had a theory that
`$state<T>()` → child expecting `T` should fire TS2322, and ran a
synthetic repro that confirmed. But upstream `svelte-check --tsgo`
didn't fire on the same file. Reading the real svelte2tsx output
showed `bind:clientWidth={mainWidth}` emits as a plain
`mainWidth = $$_div2.clientWidth;` statement — the assignment
narrows `mainWidth` via TS flow analysis. Our emit wrapped the
equivalent assignment in a never-called lambda (`void (() => { ... })`),
deliberately isolating it from narrowing. One-line unwrap, 1-FP
eliminated, bench went 1 → 0 errors. Full trace in
`crates/emit/src/lib.rs::emit_dom_binding_checks_inline` + commit
`61fb2f5`.

## When in doubt

- Read `README.md` for the public-facing overview.
- Read `ROADMAP.md` for current scoreboard, next-version plan,
  explicit out-of-scope items, and documented won't-fix limitations.
  This file is gitignored — it's the private working notes that stay
  in the local checkout only, not part of the shipped repo. Always
  treat it as the source of truth for "what's next" and "known
  limits".
- Check `language-tools/packages/svelte-check/src/` for how upstream
  solves CLI/output problems.
- Check `language-tools/packages/svelte2tsx/src/` for how the upstream
  Svelte → TS transpilation works (informs our `emit` crate).

---

## Technical reference (kept here, not in README)

### Cache layout

Cache root is chosen by `crates/typecheck/src/cache.rs::CacheLayout::for_workspace`:

1. `<workspace>/node_modules/.cache/svelte-check-native/` when
   `node_modules/` exists (gitignored everywhere by convention — same
   pattern as eslint, prettier, vite, vitest cache dirs).
2. `<workspace>/.svelte-check/` as the fallback for fresh-clone or
   no-deps fixtures.

```
<cache root>/
  tsconfig.json           overlay — extends user tsconfig, adds rootDirs,
                          paths-mirror, allowImportingTsExtensions
  tsbuildinfo.json        tsgo's incremental build state
  svelte-shims.d.ts       rune ambients, store unwrap helper, module shims
  svelte/<rel>/Foo.svelte.ts
                          generated TS per .svelte file. Imports are
                          rewritten to `.svelte.ts` so tsgo lands on the
                          overlay file rather than the *.svelte ambient
                          declaration shipped with the svelte package.
```

### Why the binary is fast

- **Single Rust process**, no per-file Node startup, no svelte2tsx
  subprocess per check.
- **Multi-worker JS bridge.** N `bun`/`node` subprocesses (default
  `cores/2`, capped at 8, override via `SVN_BRIDGE_WORKERS=N`) each
  import `svelte/compiler` once and process a chunk of files in
  parallel. Sweet spot empirically tracks the perf-core count on Apple
  Silicon — over-subscribing past it costs more in scheduler/IPC
  contention than it saves in serial work.
- **OXC for JS/TS parsing.** AST construction ~10× faster than swc and
  ~50× faster than the typescript parser.
- **rayon for the per-file parse → analyze → emit loop.** Pure compute,
  no shared state, scales linearly.
- **Incremental tsgo via tsbuildinfo.** Only changed files get re-typed
  across runs.

### Output formats (byte-spec)

All four match upstream svelte-check byte-for-byte (modulo timestamp
prefix). Editor extensions / CI dashboards / shell wrappers consuming
`svelte-check`'s output work unchanged.

`machine`:
```
1776349615385 START "/path/to/workspace"
1776349615386 WARNING "src/lib/X.svelte" 22:5 "..."
1776349615386 ERROR "src/lib/Y.svelte" 8:3 "..."
1776349615387 COMPLETED 1206 FILES 0 ERRORS 44 WARNINGS 15 FILES_WITH_PROBLEMS
```

`machine-verbose`:
```
1776349615385 START "/path/to/workspace"
1776349615386 {"type":"WARNING","filename":"src/lib/X.svelte",
               "start":{"line":21,"character":4},"end":{"line":21,"character":13},
               "message":"...","code":"state_referenced_locally",
               "codeDescription":{"href":"https://svelte.dev/docs/svelte/compiler-warnings#..."},
               "source":"svelte"}
1776349615387 COMPLETED 1206 FILES 0 ERRORS 44 WARNINGS 15 FILES_WITH_PROBLEMS
```

`<N> FILES` denominator semantic matches upstream's
`|emit entries ∪ files with diagnostics|`. "Emit entries" means every
`.svelte` file plus every SvelteKit Kit file (route modules like
`+page.ts` / `+layout.server.ts` / `+server.ts`, hooks files under
`src/hooks.*`, params under `src/params/`). Detection lives in
`crates/cli/src/kit_files.rs` and is locked against upstream's
`isKitFile` via the `kit_file_parity` integration test.

`human` is the colored compact form (file:line:col + Error/Warn label +
message). `human-verbose` (default) adds a banner prelude and a 3-line
code frame around each diagnostic with caret underlines, in cyan.

`machine` is forced when any of `CLAUDECODE=1`, `GEMINI_CLI=1`,
`CODEX_CI=1` is set in the environment.

### Exit codes

- `0` — no errors (and no warnings if `--fail-on-warnings`)
- `1` — errors detected (or warnings with `--fail-on-warnings`)
- `2` — invocation error (bad flag, missing tsconfig, missing tsgo)

### Missing flags (intentionally)

- `--watch` / `--preserveWatchOutput` — out-of-scope, won't implement.
- `--no-tsconfig` — reserved, errors out today.
- `--diagnostic-sources css` — hard-rejected (exits 2 with a hint).

### CSS rejection rationale

Upstream `svelte-check` runs CSS diagnostics through PostCSS-style
linting. We don't ship a CSS linter, and silently doing nothing when
the user explicitly asks for CSS coverage is a worse failure mode than
telling them upfront. When `--diagnostic-sources` is omitted the
default is `js,svelte` (NOT `js,svelte,css`), matching what the binary
actually produces.

### Test corpora and baselines

Two parity bars in `crates/cli/tests/`:

1. **63 `.v5` fixtures** from `language-tools/packages/svelte2tsx/test/svelte2tsx/samples/*.v5/`.
   For each, generate the overlay TS, hand it to tsgo, compare diagnostic count.
2. **24 store-pattern fixtures** locally authored exercising
   `$store` auto-subscribe through scoped/destructured/re-exported/
   external-imported store boundaries (upstream coverage is sparse).

A handful of upstream fixtures test **verbatim emit fidelity** rather
than type correctness: they contain intentionally broken user code (an
undefined ref, a mismatched generic, an import from nowhere) and the
test passes when our overlay preserves that user code character-for-
character so tsgo reports the SAME error a real user would.

`crates/cli/tests/v5_fixtures/baselines.json` declares an expected
`max_errors` count and a `reason` for those:

```jsonc
{
    "verbatim_emit_fixtures": {
        "runes-best-effort-types.v5": {
            "max_errors": 1,
            "reason": "let { g = foo } = $props() — `foo` is undefined; the fixture
                       preserves the user's reference verbatim. svelte2tsx emits
                       the same error in expectedv2.ts."
        }
    }
}
```

A baselined fixture passes if `errors ≤ max_errors`. Non-baselined
fixtures must produce zero errors. Catches two regression classes:

- A fixture that should be clean starts producing errors → fails on the
  zero-error rule.
- A baselined fixture starts producing MORE errors than its cap →
  fails on the count rule.

The `max_errors` mechanism is interim; future work replaces it with
exact `{code, line, column, message}` assertions per expected error so
a regression that swaps one error for a different one (silent today)
gets caught too.

### Recommended CI invocation

```sh
svelte-check-native \
  --workspace . \
  --tsconfig ./tsconfig.json \
  --output machine \
  --threshold warning \
  --fail-on-warnings
```

Grep `^.* COMPLETED ` for the summary line, or pipe
`--output machine-verbose` into `jq` for full structured access.

The compiler-warning bridge silently no-ops if `bun`/`node` isn't on
`PATH`. Force it OFF explicitly with `--diagnostic-sources js`.

### Release workflow

v0.3.0 process, documented for the next bump.

**Six packages ship together:**
- `svelte-check-native` — the meta/wrapper npm package users install.
- `svelte-check-native-darwin-arm64` — M-series Mac native binary.
- `svelte-check-native-darwin-x64` — Intel Mac native binary.
- `svelte-check-native-linux-arm64` — ARM Linux native binary.
- `svelte-check-native-linux-x64` — x86_64 Linux native binary.
- `svelte-check-native-win32-x64` — Windows native binary.

The meta package `optionalDependencies` pins each platform package
at the exact same version so `npm install svelte-check-native` gives
each user their correct binary. `scripts/prepare-release.mjs` is the
single source of truth that keeps the six `package.json` versions +
`optionalDependencies` pins in lockstep — always re-run it after any
version bump.

**Prerequisites (check once per workstation):**

```sh
rustup target list --installed | grep -E "apple|linux|windows"
# expect: aarch64-apple-darwin, x86_64-apple-darwin,
#         aarch64-unknown-linux-gnu, x86_64-unknown-linux-gnu,
#         x86_64-pc-windows-gnu
# add missing with: rustup target add <triple>

zig version && cargo zigbuild --version
# install with: brew install zig && cargo install cargo-zigbuild

npm whoami
# must return a user with publish rights to @harshmandan/*
```

**Bump + tag:**

1. Update `Cargo.toml` `[workspace.package].version` + root `package.json` `version` (both must match exactly).
2. Update `CHANGELOG.md` — add a new `## [X.Y.Z]` section at the top with what shipped.
3. Commit: `git commit -am "release: vX.Y.Z"`.
4. Tag: `git tag vX.Y.Z`.

**Publish:**

```sh
# Dry run — builds all platform binaries, regenerates dist-packs/,
# packs + validates without writing to the registry. Use this FIRST.
npm run publish:dry

# If dry-run is clean, publish for real. Order is enforced by
# scripts/publish-all.mjs: 5 platform packages first, then the
# meta wrapper last so optionalDependencies are resolvable when
# the wrapper hits the registry.
npm run publish:all
```

`publish:all` chains `build:all` → `prepare-release` → `publish-all.mjs`.
It does NOT push to git — push `main` + the new tag separately:

```sh
git push origin main
git push origin vX.Y.Z
```

**Post-release verification:**

```sh
npm view svelte-check-native version
# expect: X.Y.Z

# Fresh-install smoke test
cd /tmp && rm -rf sc-smoke && mkdir sc-smoke && cd sc-smoke
npm init -y
npm i -D svelte-check-native @typescript/native-preview
npx svelte-check-native --version
# expect: svelte-check-native X.Y.Z
```

**Don't:**
- Don't run `npm publish` manually in individual package dirs — ordering
  must be enforced (platforms first, wrapper last) or installs break.
- Don't bump version without re-running `prepare-release.mjs` — the six
  package.json versions will drift out of sync and the wrapper's
  `optionalDependencies` will point at nonexistent platform versions.
- Don't push a git tag before the corresponding npm publish completes —
  users grabbing tarballs by tag will see a version that isn't yet
  installable.
