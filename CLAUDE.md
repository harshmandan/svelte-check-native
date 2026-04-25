# Working conventions for Claude Code / AI-assisted contributions

This file is loaded into every Claude Code session in this repo. Read
`README.md` for the public-facing overview, and `notes/` (gitignored)
for the current scoreboard (`notes/ROADMAP.md`), open work
(`notes/OPEN.md`), bench fleet (`notes/BENCH.md`), deferred items
(`notes/DEFERRED.md`), and chronological decision log
(`notes/HISTORY.md`). This file is the shorter "rules of engagement"
layer on top of all of them.

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
- **Strictness:** We are not stricter or lax-er than the upstream. The
  goal of this project is to remain at parity with upstream. Not
  compete with it. Parity means same errors, same warnings and same
  number of problematic files.

## Scope discipline (repeated here because it's easy to forget)

Out of scope — do NOT implement:

- LSP server / editor integration
- Autocomplete, hover, go-to-definition, rename, code actions
- Watch mode (use `watchexec` externally)
- tsc fallback (tsgo only)
- Formatting
- CSS lint rules beyond the narrow vendor-prefix carve-out in ROADMAP

Svelte-4 compat isolated: every Svelte-4-specific helper goes under
`crates/*/src/svelte4/` with a `// SVELTE-4-COMPAT` marker at each
callsite. When Svelte 4 is officially retired the removal is
mechanical — delete the submodule and grep for the marker.

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

2. **Two-phase transformer.** Phase 1 (analyze) produces a set of
   per-concern structs that Phase 2 (emit) reads read-only. Never
   mutate these during emit. Never register new names during emit.

   Analyze outputs today: `PropsInfo` (props.rs), `TemplateSummary`
   (template walker, includes `SlotDef[]` for the slot-let port),
   `VoidRefRegistry` (see rule #3), plus free-function helpers
   (`collect_top_level_bindings`, `find_store_refs`,
   `find_template_refs`, `collect_typed_uninit_lets`).
   Direction of travel is centralising into a single `SemanticModel`
   as each concern gets its second consumer — don't invent placeholder
   fields with no reader.

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
   - `push_str`.
6. **Synthesized-name prefix:** `__svn_*`. Used for every name the emit
   crate creates so they're trivially distinguishable from user code in
   diagnostics.
7. **Component instantiations emit as `new $$_CN({target, props})`
   through the `__svn_ensure_component` wrapper.** Each `<Comp ...>`
   emits as:

   ```ts
   { const __svn_CN = __svn_ensure_component(Comp);
     const __svn_inst_N = new __svn_CN({ target: __svn_any(), props: { ... } });
     __svn_inst_N.$on("click", handler);  // one per on:event directive
   }
   ```

   The intermediate `const __svn_CN = ...` is load-bearing — it's
   what lets TS bind generic components' `<T>` at the `new` site
   against concrete prop values. Dropped local → `T` resolves to
   `unknown` and snippet arrows fire implicit-any.

   `__svn_inst_N` (the instance local) is hoisted only when needed
   downstream — any of: an `on:event` directive (`$on(...)`),
   `bind:this={x}` (assignment to user variable), or a slot-let
   consumer wrapper that needs the parent's `$$slot_def[name]`.
   `$on`'s signature (`handler: (...args: any[]) => any`) gives the
   arrow contextual typing so `({detail}) => …` destructures without
   implicit-any.

   Overlay default exports use the `$$IsomorphicComponent` interface
   pattern (post-iso-port, since v0.6 — see commit `8de67ec0`):

   ```ts
   interface $$IsomorphicComponent {
     new (options: ComponentConstructorOptions<P>): SvelteComponent<P, E, S> & { $$bindings?: B } & X;
     (internal: unknown, props: P): X & { $set?: any; $on?: any };
     z_$$bindings?: B;
   }
   const __svn_component_default: $$IsomorphicComponent = null as any;
   type __svn_component_default = InstanceType<typeof __svn_component_default>;
   export default __svn_component_default;
   ```

   Where P/E/S/B/X project from `Awaited<ReturnType<typeof
   $$render>>['props' | 'events' | 'slots' | 'bindings' | 'exports']`.
   Mirrors upstream svelte2tsx `addComponentExport.ts:170-179`
   byte-for-byte. The `& { $set?: any; $on?: any }` on the callable
   is what makes our return assignable to a user-declared bare
   `Component<{}>` target.

   `__svn_ensure_component` is a single conditional-return overload
   (also post-iso-port) that returns `T` directly when T is newable,
   and synthesises a constructor when T is callable
   (`Component<P, X, B>`).

   For generic components with a Props type source, the render
   function is wrapped in `class __svn_Render_<hash><T> { props()
   { ... } events() { ... } slots() { ... } bindings() { ... }
   exports() { ... } }` (gated on
   `use_class_wrapper = generics.is_some() && prop_type_source.is_some()`).
   Mirrors upstream's `__sveltets_Render<T>` pattern so body-local
   type refs in the Props type resolve inside the render scope
   rather than leaking to module scope (TS2304). When generics are
   present without the class wrapper, the `$$ComponentProps` type
   alias emits INSIDE the render fn body so the generic binder is
   in scope.

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
`expectedv2.js` pattern against _our_ emit. `UPDATE_SNAPSHOTS=1
cargo test --test emit_snapshots` accepts deliberate emit changes;
default mode fails on any mismatch with a contextual diff. ~410
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
upstream test update applied for free. A handful of known-failing
SvelteKit-ambient-typing cases remain (tracked in `notes/OPEN.md`
when actionable); the bulk passes.

**Discovery (not tests).** Real-world repos in `bench/` are _not_
part of `cargo test`. They're used interactively to find bug classes
that get extracted into new `bug_fixtures/<NN>-*` entries and locked
by the suites above. Their error counts are not a shipping metric.

**Bench targets for perf measurement (`scripts/bench.mjs`):**

- A Svelte-4 control-rig bench — the 1000-file parity-gate
  target. Mid-migration SvelteKit monorepo, mostly Svelte-4-
  syntax components. The "1000-file mid-migration" number in the
  public README and CHANGELOG refers to this workspace's primary
  sub-app (~1124 files after monorepo-root auto-escape). Ties
  upstream `svelte-check --tsgo` at 0 user errors.
- A Svelte-5 control-rig bench — the latest fresh extract of the
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
- **`cargo test` is the scoreboard.** Bench parity is the user-facing
  scoreboard — see `notes/BENCH.md` for the full delta vs upstream
  `svelte-check --tsgo`.

## Diagnostic method — "diff the real upstream artifact"

**Any time our error count diverges from upstream on any file, the
FIRST move is to diff emits with upstream AND read the relevant
section of the `language-tools/` submodule source.** No synthetic
repros, no theorizing about "why TS should fire here", no speculative
Rust changes — the upstream artifact and the upstream source are the
ground truth, and reading them is almost always faster than reasoning
about them.

The debugging path:

1. **Anchor on a real failing file.** Pick the exact `.svelte` file
   from a bench or submodule test fixture where the diagnostics
   diverge. Don't build a synthetic reduction first — the real file
   captures all the context that matters (`$state()` uninit patterns,
   `bind:` combinations, destructured `$props()`, slot shapes, etc.).

2-4. **Diff upstream's overlay vs ours.** One command via
`scripts/diff-emit.mjs`:

```sh
# Side-by-side diff of upstream's overlay vs ours.
node scripts/diff-emit.mjs path/to/File.svelte

# Dump only one side.
node scripts/diff-emit.mjs path/to/File.svelte --upstream
node scripts/diff-emit.mjs path/to/File.svelte --ours

# Probe: append a type-check trap to our overlay that reveals
# what a specific identifier (typically a component or import)
# resolves to. Prints the TS2322 diagnostic that leaks the
# inferred type. Useful when a site should fire an excess-prop
# check but doesn't — tells you whether the Props type is being
# extracted correctly or is falling through to `any`.
node scripts/diff-emit.mjs path/to/File.svelte --probe "UI.Dropdown"
```

Workspace + tsconfig are inferred from the file path (walks up
for `node_modules`). `--isTsFile` / `--isJsFile` overrides the
`<script lang="ts">` auto-detection. When the workspace itself
has no `svelte2tsx` (e.g. pure npm projects), the script falls
back to any sibling `bench/*` install.

Focus on the specific LINES tsgo flags (grep the overlay for
the relevant variable names, not whole-file diffs). The
structural delta is usually a wrapper / lambda / parenthesization
/ extra intersection — not a primitive type difference.

5. **Read the upstream source that produces the shape you saw in
   step 4.** The submodule at `language-tools/` is pinned — use it.
   For emit-shape questions:

   - `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/` is the
     template → JS/TS walker. Elements, components, blocks
     (`{#each}`, `{#if}`, `{#snippet}`, `{#await}`), directives,
     bindings, actions, slots each have their own file.
   - `language-tools/packages/svelte2tsx/src/svelte2tsx/` is the
     script → overlay wrapper (render fn, props-type source,
     default-export shape).
   - `language-tools/packages/svelte-check/src/incremental.ts` is
     CLI/output glue — kit file injection, diagnostic mapping,
     `.svelte.js` vs `.svelte.ts` extension choice.

   Find the one upstream source file whose output matches the shape
   you saw in the overlay dump. That file is the spec.

6. **Lock upstream's shape as a tsgo fixture first** (Rule #8 of
   "Architecture rules"). Minimal `.ts` / `.js` files under
   `design/<topic>/` that produce exactly the expected diagnostics
   from tsgo. Commit before any Rust change. If the fixture doesn't
   reproduce the desired behavior, the theory is wrong and coding
   won't fix it.

7. **Only then port the shape into our emit.**

### When to deploy it

- Any time `cargo test --test upstream_sanity` or a bench's error
  count diverges from `svelte-check --tsgo` on the same workspace.
- Any time a theory about "why TS should fire here" conflicts with
  what `svelte-check --tsgo` actually reports.
- Any time you catch yourself writing "TS should do X here" — the
  real artifact is the ground truth; reading it is almost always
  faster than reasoning about it.

---

## Exit codes

- `0` — no errors (and no warnings if `--fail-on-warnings`)
- `1` — errors detected (or warnings with `--fail-on-warnings`)
- `2` — invocation error (bad flag, missing tsconfig, missing tsgo)

## Release workflow

Six packages ship together: `svelte-check-native` (meta wrapper) +
five platform binaries (`-darwin-arm64`, `-darwin-x64`,
`-linux-arm64`, `-linux-x64`, `-win32-x64`). The wrapper's
`optionalDependencies` pins each platform at the same version.
`scripts/prepare-release.mjs` keeps the six `package.json` versions
and pins in lockstep — always re-run after any version bump.

**Bump:** update `Cargo.toml` `[workspace.package].version` and root
`package.json` `version` (must match), add a `CHANGELOG.md` entry,
commit as `release: vX.Y.Z`.

**Publish (in order):**

```sh
npm run publish:dry       # dry run first — builds + packs, no registry writes
npm run publish:all       # real publish; enforces platforms first, wrapper last
git push origin main      # publish:all does NOT push
```

**Cut the GitHub release last.** `gh release create "v$VER"` creates
the tag on the remote pointing at main's HEAD, so tag+release stay
in sync by construction. Body: 2-3 line prose summary, blank line,
hand-curated grouped list (max 10 entries, `<sha>` or `<a>..<b>` per
bullet). Drop `release: vX.Y.Z` commits from the list.

```sh
PREV=<previous-tag>; VER=X.Y.Z
git log --pretty=format:"%h %s" "$PREV..HEAD" | grep -v "release: v"
gh release create "v$VER" --title "v$VER" --notes-file /tmp/notes.md --latest=true
```

**Post-release:** `npm view svelte-check-native version` should
return `X.Y.Z`; smoke-install in a scratch dir.

**Don't:**

- Don't `npm publish` in individual package dirs — the platforms-first
  ordering is load-bearing.
- Don't bump version without re-running `prepare-release.mjs`.
- Don't `git tag` manually — let `gh release create` do it.
- Don't create the release before `npm publish:all` and `git push` complete.
- Don't mark stable releases `--prerelease`.
