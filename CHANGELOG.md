# Changelog

All notable changes to `svelte-check-native` will be documented in this
file. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [SemVer](https://semver.org/spec/v2.0.0.html).

## [0.6.5]

Patch release. Two architectural consolidations + four review-driven
bug-fix rounds. Bench parity unchanged — all 15 control benches keep
their byte-perfect counts against `svelte-check --tsgo`; c4 + c5
control rigs stay at 0 user errors and 1 ahead of upstream (which
still reports a synthetic tsconfig diagnostic on its own injected
`incremental: false` + `tsBuildInfoFile` combination).

### Architecture

- **SvelteKit path classification centralised
  (`269c1305`..`083c11bb`).** Five sites in this repo previously
  reimplemented overlapping subsets of "is this a kit file? what
  kind?" rules: `cli/kit_files`, `cli/svelte_config`,
  `emit/kit_inject`, `emit/sveltekit`, `typecheck/kit_types_mirror`.
  Round 3 F4 + Round 4 G4-G5 each fixed one site without touching
  the others. Six phases consolidate every site behind
  `svn_core::sveltekit::classify` / `normalise_path` /
  `user_source_needles`. Future SvelteKit-convention additions
  land in one place instead of five.
- **Template-scope walker unified (`478d6928`..`740780e5`).**
  Both `analyze::template_walker` and `svn-lint::scope` walked the
  same Fragment/Node tree and tracked the same scope-introducing
  constructs (`{#each}`, `{#snippet}`, await branches, let
  directives, `{@const}`) independently. Five phases collapse the
  recursion into `svn_analyze::template_scope::walk_with_visitor`,
  driven by a `TemplateScopeVisitor` trait that both crates
  implement. ~250 LOC of duplicated walker arms retire; future
  scope-related fixes land in one place.

### Correctness

- **Round 3 (F1-F16, `f0d8e93e`..`835d5925`).** Sixteen targeted
  fixes spanning parser bugs, scope drift, cache GC, UTF-16
  diagnostic mapping, and `kit.files` parsing. Notable: void HTML
  elements no longer push template-tag depth; files-only tsconfigs
  treat as closed-world; cache GC handles deleted/renamed sources;
  diagnostic mapping converts UTF-16 columns to byte offsets.
- **Round 4 (G1-G10, `4f9562fe`..`e272104c`).** Ten cleanups:
  `runes={false}` honours literal value; array-rest captured in
  template shadow; `{@const}` retag scoped to its own bindings;
  slot-attr shadow check on member/index expressions;
  `kit_types_mirror` GC + earliest-needle match; v5 fixture
  suites assert `skipped == 0`; bench parity gates on file-count
  drift; svelte.config parsed once and shared across extractors.
- **Round 5 (`2e281d99`, `232d9791`).** Seven findings: Windows
  path normalisation in `kit_files`; JS overlays now void
  `__svn_tpl_check`; emit_snapshots harness fails loud on missing
  corpora and discovers `+page.svelte` / `+layout.svelte`; bench
  parity forwards `--tsconfig` to upstream; types-mirror stops
  rewriting hooks/params chains (kit_inject doesn't materialise
  cache copies for them); runes scanner skips strings/comments
  inside template-literal interpolations; documented `kit.files.routes`
  parity with upstream.
- **Round 6 (`3438e72a`).** Four findings inside the new
  abstractions: await-branch context bindings declare as
  `Template` (not `Each`) so `bind_invalid_each_rest` doesn't
  misfire on `{:then {...rest}}`; destructure `{@const}` forms
  participate in analyze's shadow tracking; let-directive default-
  value expressions walk in parent scope; `classify` tolerates
  non-UTF-8 path components.

### Bug fixtures

Seven new locked fixtures in `fixtures/bugs/`:
`55-sveltekit-route-autotyping`, `56-sveltekit-layout-route-autotyping`,
`69-at-const-shadows-slot-attr`, plus the six existing fixtures the
emit-snapshots harness picked up after gaining `+page.svelte` /
`+layout.svelte` discovery. Snapshot count grows ~9.

## [0.6.1]

Patch release. Performance, code-quality, and architecture cleanups
across the workspace. Parity unchanged — c4 + c5 control benches
stay at 0 user errors and ahead of upstream `svelte-check --tsgo`
(which still over-reports by 1 error on each).

### Performance

- **Dropped per-file `dunce::canonicalize` in scope filter
  (`6a00af61`).** Workspace path was already canonicalized at
  startup, so the per-walker-entry canonicalize was redundant.
  Discovery phase: c4 warm 69 → 17ms (-75%), c5 warm 87 → 20ms (-77%).
- **Module script parsed once per component (`a8592307`).** Was
  parsed twice — once for `collect_top_level_bindings`, once for
  `collect_type_only_import_bindings`. Hoisted the allocator and
  AST so both consumers share a single parse.
- **Fused `@const` name collection into `walk_template`
  (`49b53198`).** Eliminates a second recursive fragment walk;
  collapses 75 LOC of duplicated traversal logic.
- **`HashSet<SmolStr>` instead of `HashSet<String>` for store/type
  ref dedup (`dfa9911a`).** Per-template-ref `to_string()` allocs
  replaced with `SmolStr` clone-bumps.
- **HashSet replaces O(n²) Vec dedup in overlay paths merge
  (`88f8d505`).**

### Code quality

- **`main.rs` split into focused modules (`abe9c32c`).** Discovery,
  output formatting, and `.svelte`-import collision rewriting moved
  to their own files; main.rs shrinks 2072 → 1446 LOC (-30%).
- **`core::walk_up_dirs` helper (`df4649f1`).** Six hand-rolled
  `while let Some(dir) = cur { … cur = dir.parent() }` loops in
  cli, typecheck, svn-lint, and svelte-compiler collapse onto a
  single closure-driven helper.
- **`core::synth_names` module (`e5e7c992`).** Single source of
  truth for `__svn_C_<hex>`, `__svn_inst_<hex>`, and the
  `__svn_tpl_check` constant — was scattered across 7 sites in
  emit and analyze.
- **`core::NODE_MODULES_DIR` const (`62591967`).** Replaces 10
  inline `"node_modules"` literals across path-construction sites.
- **Visibility hygiene across emit + svn-lint (`3f553e16`,
  `0261472d`).** `pub mod` → `mod` everywhere the items had no
  external callers; surfaced and removed three dead helpers
  (`scope::build`, `find_plain_attr`, `plain_attr_text`) plus an
  empty `analyze::svelte4` placeholder module and three unused
  `pub use oxc_*` re-exports in parser.
- **Emit-shape collapses.** Class-wrapper method emission loops
  five fields (`props` / `events` / `slots` / `bindings` /
  `exports`) instead of five `writeln!` chains (`ae2e8394`);
  `$$IsomorphicComponent` generic and non-generic arms collapse
  via a `g_prefix` substitution (`98bee54b`);
  `emit_default_export_declarations` splits into `_js` (2 params,
  pure JSDoc) and `_ts` (9 params, full TS machinery) —
  eliminates the six `let _ = X;` JS-branch sentinels that
  signalled the function was two functions stuck together
  (`c669fdfe`).

### CLI

- **Dropped `--no-tsconfig` and `--ignore` flags (`d001049d`).**
  `--no-tsconfig` was parsed but errored at runtime; `--ignore`
  was gated on it. Both unreachable in practice. The plumbing
  removal also dropped the `ignore: Option<&GlobSet>` parameter
  threaded through discovery + `run_typecheck`.

### CI

- **Dependabot + `cargo-deny` for supply-chain hygiene
  (`e889e4e8`, `f0723dd2`).** Cargo, npm, and GitHub Actions
  ecosystems on a weekly cadence; license/advisory gating via
  cargo-deny.

## [0.6.0]

Minor release. Closes the entire previously-tracked parity surface:
**10 / 15 benches now at byte-perfect parity** with upstream
`svelte-check --tsgo` on errors+warnings+files-with-problems. The
remaining 3 deltas reduce to one user-side type-unsoundness pattern
(palacms recursive `<Item>` self-reference), one upstream-side
synthetic-tsconfig diagnostic on its own injected file, and two
bench-config issues both sides hit differently.

### Emit — major ports

- **Slot-let consumer port (`8c81f13d`).** `<Inner slot="X" let:foo>`
  now destructures from `parent_inst.$$slot_def["X"]` at the parent's
  child-walk depth (BEFORE Inner's component-call) — `foo` is in scope
  for Inner's own `$on(...)` handler and children. Producer side:
  `<slot {x}={expr}>` flows typed entries through `$$render`'s `slots:`
  literal. Mirrors upstream svelte2tsx's `InlineComponent.ts:184-207`
  byte-for-byte.

- **Slot-let extends to `<svelte:fragment>` + DOM elements
  (`d3f3177f`).** Same wrap mechanics now accept all three element-shape
  kinds via a unified `slot_let_attrs` helper.

- **Isomorphic-component port stages 1+2+3 (`8de67ec0`).** Default-export
  emit unified to `$$IsomorphicComponent` interface for every component;
  `__svn_ensure_component` collapsed to one conditional-return overload;
  `$$render`'s `events:` field CustomEvent-wrapped.

- **`$set?: any; $on?: any` on iso-interface callable (`fd126e98`).**
  Matches upstream's `__sveltets_2_IsomorphicComponent` shape. Closes
  +4 `Component<{}, {}, string>` assignability over-fires.

### Emit — overlay correctness

- **`$$ComponentProps` annotation collapse preserves source line count
  (`d9a0d98d`).** Previously the `}: { multi-line literal } = $props()`
  rewrite collapsed to a single-line `: $$ComponentProps`, drifting
  every declaration after the destructure by `(literal-line-count - 1)`
  source lines in mapped diagnostics. Now pads newlines INSIDE the
  ignore-marker span so source line count is preserved.

- **`$$ComponentProps` alias in render-body scope when generics declared
  (`364f1078`).** `<script generics="T">` declares T on the render fn's
  binder. A Props literal referencing T (e.g. `Snippet<[LoadedPopup<T>]>`)
  emitted at module scope as `type $$ComponentProps = { … T … }` then
  fires TS2304. Move the alias INSIDE the render body. Closes 2
  `Cannot find name 'SomePopup'` over-fires.

- **`{@attach EXPR}` directive emit shape (`364f1078`).** Previously
  emitted as `...EXPR,` (re-using the spread shape). For arrow
  expressions like `{@attach el => el.focus()}` this left the parameter
  as implicit-any → TS7006. Switch to upstream's
  `[Symbol("@attach")]: EXPR,` form. The `[key: symbol]: Attachment<T>`
  index signature on `HTMLAttributes` types the arrow's parameter as
  the element type. Closes 3 implicit-any over-fires.

- **TokenMapEntry per component prop + lax events fallback
  (`bd0d0b7f`).** Closes FieldItem position drift +
  event-handler `detail` over-fires.

- **`$$ComponentProps` annotation reference (`51b54b50`).** Replaces
  multi-line literal annotation with a single `$$ComponentProps`
  reference. Line-count parity with upstream.

- **Component-call brace open across child walk (`cb96b0dd`).** Each
  component-call's `{ … }` block scope now stays open across child
  walks. `{@const X = …}` declarations live in the component's block
  scope; sibling components each have their own X without TS2451
  redeclare collisions.

### SvelteKit + tsconfig

- **Arrow-form `load` handler injection (`8495615`).** `+page.ts` /
  `+layout.ts` / `+page.server.ts` / `+layout.server.ts` arrow-form
  `load = async ({event}) => {…}` now receives the typed
  `PageLoadEvent` / `LayoutServerLoadEvent` etc. injection. Closes
  `+page.ts` Kit route-type gap.

- **Skip arrow-form load injection when user typed `load: Load`
  (`c156cc22`).** Splicing the narrower Kit-route event type onto an
  arrow already constrained to the broader `Load` signature created a
  contravariant param mismatch. Honour the user's explicit annotation
  via `declarator.id.type_annotation.is_none()`.

- **Drop forced `skipLibCheck=true` overlay override (`2fa6be5`).** Per
  CLAUDE.md ("not stricter or lax-er than upstream"), honour the user's
  tsconfig setting. Surfaced cryptgeon's `Invalidator` removed from
  `svelte/store` in `@zerodevx/svelte-toast`. Side-effect
  fix: removed an invalid `'*/css/*'` pattern from our own
  `svelte_shims_core.d.ts` (TS module patterns allow at most one `*`).

- **Drop forced `forceConsistentCasingInFileNames=false` overlay
  override (`ad98a966`).** Same rationale. Zero bench impact (the TS1149
  it dodged doesn't surface in real-world cache-mirror layouts).

### Diagnostic mapping

- **Lexical normalisation of diagnostic file paths (`6e1aab53`).**
  When tsgo emits relative paths with `..` segments, joining onto the
  canonical workspace produces a syntactically-`..`-bearing absolute
  path that misses the `map_data` HashMap lookup. Resolve `.` and `..`
  components without touching the filesystem.

### CLI

- **`--svelte-warnings native|bridge` flag (this release).** Exposes
  the native-vs-JS-bridge implementation choice. Default `native` (fast
  Rust lint port) skips `css_unused_selector`; opt-in `bridge` matches
  upstream byte-for-byte at +1.5-2s cold-start cost. |

## [0.5.0]

Minor release. Three diagnostic-correctness fixes driven by a
root-cause investigation that exposed a tsgo semantic-abort
masking errors across multiple bench workspaces, plus the
parity-push work accumulated since 0.4.2.

### Emit

- **SvelteKit `$types` injection for Svelte-4 `export let data`**
  on `+page.svelte` / `+layout.svelte` Kit-route files. Now
  injects `: import('./$types.js').PageData` (or LayoutData,
  ActionData, Snapshot) to match upstream svelte2tsx's
  `handleTypeAssertion` behavior
  (`svelte2tsx/src/svelte2tsx/nodes/ExportedNames.ts:424-440`).
  Closes a 130-error cluster on a charting-lib bench where
  `data.geojson.objects.states` was collapsing to `any` and
  routing `topojson-client`'s `feature()` overload to the
  wrong branch.

- **TS-overlay default export sources Props via
  `Awaited<ReturnType<typeof $$render>>['props']`** instead of
  the previous `Component<Record<string, any> & __SvnAllProps,
{exports}>` shape. The old shape put the actual prop type in
  the Exports slot (second generic of `Component<>`), leaving
  the Props slot as a loose record — which broke contextual
  typing on arrow-callback props at consumer sites. New shape
  routes through the render function's return object, matching
  upstream's `__sveltets_2_isomorphic_component($$render())`
  pattern. Restores `(e: MouseEvent) => void` contextual typing
  on patterns like `<Bar onclick={(e) => e.clientX} />`.

- **JS-overlay Svelte-4 ambient declarations use JSDoc casts
  instead of TS annotations.** Previously emitted `let $$props:
Record<string, any> = {};` directly into `.svn.js` overlays
  — tsgo fires TS8010 ("Type annotations can only be used in
  TypeScript files") on every such overlay AND silently
  aborts the project-wide semantic-check pass once TS8010
  surfaces anywhere. Now emits `let $$props = /** @type
{Record<string, any>} */ ({});`. Unblocks full semantic
  checking on projects with SVG icon libraries or other
  Svelte-4 `.js` component libraries — exposed real errors
  that had been silently suppressed.

- **Leading `@ts-ignore` / `@ts-expect-error` / `@ts-nocheck`
  hoists with its import.** Previously, when our overlay
  hoisted an `import QR from 'qrious'` statement with a
  preceding `// @ts-ignore`, the directive was stranded in
  the body and no longer suppressed its target error. The
  hoist now extends backwards through contiguous directive
  comments. Matches upstream `svelte2tsx`'s behavior (see
  `test/svelte2tsx/samples/import-leading-comment/`).

- **Leading `/** @ts-ignore \*/` handled\*\* across the whole
  import-leading-comment test family; snapshots reflect the
  corrected hoist shape.

### Typecheck — ignore-region diagnostic filter

- **New `/*svn:ignore_start*/…/*svn:ignore_end*/` marker
  pair.** Emit can wrap scaffolding bytes with the markers
  and the mapper drops any diagnostic whose overlay byte
  position falls inside a marked region. Mirrors upstream
  `language-server/src/plugins/typescript/features/utils.ts:86-109`
  `isInGeneratedCode`. First adopter site:
  `emit_component_bind_widen_trailers` — the
  `void (() => { target = __svn_any(null); })` scaffolding
  emitted after component instantiations with `bind:X`
  widen targets. More emit sites can adopt the markers
  incrementally.

### Parity push (cumulative since 0.4.2)

- JS-overlay default for `.svelte` without `<script lang="ts">`.
  JSDoc `@typedef Props` flows through `Awaited<ReturnType>`.
- `$$ComponentProps` synthesis (JS and TS) for `$props()`
  destructure without annotation. Per-key type inference
  from default values + user JSDoc Props.
- Class-wrapper emit: `$$IsomorphicComponent`,
  `__sveltets_2_isomorphic_component`,
  `__sveltets_2_with_any_event` wrapping. Props type refs
  resolve in render scope.
- `style:X` directive type-checked via
  `__svn_ensure_type(String|Number, expr)` pin.
- `bind:X` widening on simple-identifier component targets.
- Token-map coverage for shorthand DOM attrs, `{#each}` expr,
  hoisted JS-overlay imports.
- Blanket TS2695 filter removed; upstream-matching fires
  through.
- Conditional-distribution `__svn_ensure_component` — class
  passthrough first so generics bind; callable-form second
  for our overlays.

### Documentation & infrastructure

- Bench repo names scrubbed from source comments, design
  fixture READMEs, and CHANGELOG. Generic descriptors
  throughout (e.g. "a charting-lib bench", "our Svelte-4
  control"). Private working notes unaffected (gitignored).
- `classroomio` bench removed from the fleet (its tsconfig
  extends chain depends on `svelte-kit sync` output that
  isn't checked in; upstream can't run cleanly against it).

## [0.4.2]

Patch release. Closes the long-standing `.svelte` + `.svelte.ts`
sibling-collision bug (kampsy-ui / shadcn-svelte style component
libraries), lands a substantial internal emit refactor with zero
behavior change on the three parity-gate benches, and removes one
broken escape-hatch env var. No breaking changes; no new CLI
surface.

### Emit / overlay

- **`.svelte` + `.svelte.ts` sibling collision fixed.** When a
  user `.ts` file does `import X from './Foo.svelte'` and
  `Foo.svelte` sits next to a sibling `Foo.svelte.ts` runes
  module (Svelte 5 convention used by shadcn-svelte, bits-ui,
  kampsy-ui, and similar), tsgo's `rootDirs` resolution strips
  `.svelte`, checks `Foo.d.svelte.ts` (ours, in cache) — doesn't
  find it — then falls through to `Foo.svelte.ts` via bundler
  auto-extension, landing on the runes module. The runes module
  has named exports but no `default`, firing TS2305 "has no
  exported member 'default'".

  Fix: emit a mirror overlay of the user's `.ts` file at the
  cache path (same pattern as SvelteKit Kit-file overlays), with
  every collision-case `.svelte` specifier rewritten from
  `./Foo.svelte` → `./Foo.svelte.svn.js`. Bundler module
  resolution then resolves straight to the cache-side
  `Foo.svelte.svn.ts` overlay without the auto-extension
  fallthrough. Scoped narrowly: a rewrite only fires when BOTH
  `Foo.svelte` AND `Foo.svelte.ts` are present as siblings —
  standalone `.svelte.ts` runes modules (no component) resolve
  unchanged.

  Bench deltas: a UI-library bench 8 errors → 0 (exact match with
  upstream); a desktop-app bench -3 vs tsgo → 0 (exact match with
  upstream). No
  user source tree writes; no pollution of
  `allowArbitraryExtensions`-less tsconfigs or language-server
  consumers.

### Internal refactor (no behavior change)

Major cleanup of the emit crate, landed across 14 commits
between `4484c83` (phase 4.2.1) and `86e5fca` (default-export
extraction). Emit now uses a single `EmitBuffer` position-
tracking primitive instead of threading `String` +
`Vec<LineMapEntry>` + `Vec<TokenMapEntry>` through 20+ entry
points. Every TokenMapEntry is pushed at emit time (at the exact
byte position of the splice) instead of being reconstructed by
a post-walk scanner.

- **All six post-walk `collect_*_token_map` scanners deleted.**
  `collect_interpolation_token_map` (~140 LOC,
  paren-balance + sentinel lookup),
  `collect_action_directive_token_map` (~90 LOC,
  `const __svn_action_N =` + `__svn_map_element_tag(` +
  paren-balance), `collect_on_event_token_map` (~95 LOC,
  `__svn_inst_<hex>.$on(...)` + inst cursor),
  `collect_dom_binding_token_map` (~70 LOC, MID-anchor backtrack),
  `collect_bind_this_check_token_map` (~55 LOC, `/* bind:this */`
  anchor), `collect_satisfies_token_map` (~75 LOC,
  `new __svn_C_<hex>` + paren-balance). Replaced wholesale with
  `buf.append_with_source(text, source_range)` at each splice
  site (and `buf.push_token_map(...)` for the synthesized
  component-call span). ~530 LOC of character-level scanner
  code deleted.

- **Related markers removed.** The `/*svn_I*/` interpolation
  sentinel is gone (no more scanner anchoring, no more
  paren-balance). The `/* bind:this */` marker comment is gone
  (same reason).

- **DOM-element bind emit paths unified.**
  `emit_dom_binding_checks_inline` (`bind:value`, `bind:checked`,
  `bind:files`, `bind:group`, `bind:clientWidth`, …) and
  `emit_bind_this_element_check_inline` merged into one loop
  dispatching the target-type per directive name — both already
  emitted the same `EXPR = null as any as TYPE;` shape after
  the earlier direct-assignment alignment. 123 → 69 LOC in the
  emit block.

- **Default-export declarations extracted.**
  `emit_default_export_declarations` now owns the class-wrapper
  `declare class`, the 4-arm `declare const __svn_component_default`
  / type alias pair, the `type __svn_tpl_type_refs = [...]`
  alias, and the final `export default`. Drops
  `emit_document_with_render_name` from ~1110 LOC → 763 LOC
  toward the 400-LOC goal; default-export helper is self-
  contained in 239 LOC.

- **`state_referenced_locally` compat gate baked into `Binding`.**
  Previously the rule consulted `ctx.compat` at every binding
  (multiple version gates — `state_locally_fires_on_props`,
  `state_locally_rest_prop`, plus kind + reassigned + primitive-
  init). The resolved boolean is now pre-computed as
  `Binding::fires_state_referenced_locally` at scope-build time
  by a new `populate_compat_gated_fields` pass. The rule reads
  one flag; the scope tree is now version-invariant for this
  question.

### Removed

- **`SVN_TSGO_BUILDERS` env var removed.** Previously plumbed as
  a tuning knob alongside `SVN_TSGO_CHECKERS` and
  `SVN_TSGO_SINGLE_THREADED`. Setting it produced **fake
  speedups** of 3–6× on both cold and warm runs by silently
  hiding errors: `--builders` is a `tsgo --build` (project-
  references) flag, but our single-project invocation mode
  (`--project <overlay>`) rejects it with TS5093 and exits
  without diagnostics. The wrapper then treated the empty
  output as "0 errors", so any user who set the env var got
  zero-error runs across their entire workspace. Fixed by
  removing the plumbing entirely; if tsgo `--build` mode is
  ever adopted here, the flag comes back. `SVN_TSGO_CHECKERS`
  and `SVN_TSGO_SINGLE_THREADED` stay — both valid in single-
  project mode.

### Performance investigation (no code change)

Profiled cs-4 and cs-5 cold + warm with `samply` @ 4 kHz.
Findings captured in `notes/ts7-tracking.md`:

- **tsgo dominates.** 95% cold / 84% warm / 83% dirty of
  wall-clock on cs-5. Our Rust code consumes ~150–200 ms total
  in every scenario — a rounding error.
- **Parallelism underutilised.** user/real ~3× on 8-core; 5
  cores effectively idle. Not CPU-bound.
- **tsgo's own breakdown** (cs-5 warm, 1.28M lines across 8106
  files): Parse 568 ms, Check 4 ms (incremental), Changes
  compute 286 ms. Parse is the floor.
- Conclusion: no easy wins without tsgo-side changes; our
  overlay shape already uses `skipLibCheck`, incremental
  tsbuildinfo, scoped `include`. Content-addressable emit cache
  might save ~50 ms warm but adds infrastructure.

### Bench state at release

| Bench                          | Files | Errors | Warnings | FWP | vs upstream                                                                         |
| ------------------------------ | ----: | -----: | -------: | --: | ----------------------------------------------------------------------------------- |
| our Svelte-4 control admin-app |  1124 |      0 |        2 |   2 | exact parity                                                                        |
| our Svelte-5 control admin-app |  1357 |      0 |       49 |  17 | exact parity                                                                        |
| a charting-lib bench           |   348 |    211 |        0 |  59 | -1 file; +185 vs tsgo (see `notes/OPEN.md` — blocked on structural tsgo constraint) |
| a UI-library bench             |   183 |      0 |       30 |  17 | exact parity (was +8 errors pre-fix)                                                |
| a desktop-app bench            |   206 |      0 |        0 |   0 | exact parity (was -3 errors pre-fix)                                                |

## [0.4.1]

Patch release: error-side parity improvements targeting TS-emit-shape
gaps that `svelte-check --tsgo` handled and we didn't. No breaking
changes.

### Emit (Svelte 4 callable shape)

- **Contextual typing restored on callback-shaped prop values.**
  When a Svelte-4 `interface $$Props { foo?: typeof foo; … }`
  referenced body-local `typeof` targets, the callable-form
  default export collapsed its props slot to
  `Partial<Record<string, any> & …>`. Consumer sites like
  `<Foo labels={{ format: (value) => … }}/>` then lost
  contextual typing on the arrow parameter and fired TS7006
  implicit-any at every such destructure. The sanitized exports
  record now backs the props slot when the component uses
  `export let` (Svelte-4 `export let foo` is both a prop and an
  exported member — one record describes both). Runes components
  with `export function` methods keep the permissive fallback
  since their exports are methods, not props.

- **Arrow-signature synthesis for untyped function-valued
  `export let`.** `export let fn = (a: number) => …` now
  synthesizes `fn?: (a: number) => any` in the Props type
  instead of collapsing to `fn?: any`. Consumers passing arrow
  callbacks to these props keep their parameter typings.

- **ASI scan accepts multi-line type annotations.** Multi-line
  union annotations on `export let` were tripping
  `rewrite_definite_assignment_in_place`'s ASI scan — it broke
  at the first depth-0 newline inside the type and missed the
  trailing `= undefined` initializer, producing a `!` on a
  declarator that already had an initializer and firing TS1264.
  Scan now allows a depth-0 newline when the next non-whitespace
  is a union / intersection continuation.

### Overlay (project references)

- **Sibling projects' `compilerOptions.types` now flows through
  the overlay.** In workspaces that pull files from a referenced
  project via the sibling-include union, ambient-type entries
  declared only on the referenced project's tsconfig (e.g.
  `@types/chrome` on an extension sub-package) weren't reaching
  tsgo when those files were type-checked — producing TS2304
  "Cannot find namespace" and implicit-any on declarations
  written against the missing ambient. `FlattenedReference` now
  carries each reference's `types` + `lib` from its own extends
  chain; the overlay unions sibling `types` into the user
  workspace's entries when the user explicitly set theirs,
  filtered against `is_resolvable_types_entry` per sibling's
  project dir so unresolvable entries can't fatally TS2688.

## [0.4.0]

Minor release built on two themes: a native Rust compile-warning
lint replacing the Node/Bun bridge as the default, and a
cross-bench parity push that brought 13 of 16 bench workspaces
to exact Svelte-side-warnings parity with upstream `svelte-check`.

### Headline

- **`svn-lint` crate (new).** Native Rust port of
  `svelte/compiler`'s compile-warning pass. 66 of ~80 upstream
  warning codes implemented, 118/129 upstream validator fixtures
  enforced. Scope tree, a11y role tables, `svelte-ignore` stack,
  and `warningFilter` Tier 1 + Tier 2 (static analysis of
  `svelte.config.js`) all ship with it.
  `--svelte-warnings=native` is the default — no `bun`/`node`
  subprocess, warm-run wall-clock drops ~65% on benchmark
  workspaces vs bridge. `bridge` / `both` remain available for
  opt-in / parity testing.

### Version-gated rule behavior

The native pass reads the user's `node_modules/svelte/package.json`
at startup and gates rules whose behavior changed between svelte
minor releases:

- `a11y_pointer_touch_handlers` (svelte ≥ 5.48.3).
- `state_locally_fires_on_props` (svelte ≥ 5.45.3).
- `state_locally_rest_prop` (svelte ≥ 5.51.2).

Workspaces on older svelte see matching older rule behavior;
newer workspaces get the fixture-tested current ruleset. Adding
new gates is straightforward — see `crates/svn-lint/src/compat.rs`.

### Parity fixes (scope walker)

Reference-tracking holes that made props look unused / flagged
free-variable reads that were actually bound:

- `Expression::ChainExpression` now visited (was silently
  skipped — identifiers inside `obj?.foo(x)` were invisible).
- `ArrayExpressionElement::SpreadElement` and
  `ObjectPropertyKind::SpreadProperty` arguments walked.
- `class:foo` / `style:foo` / `use:foo` / `transition:foo` /
  `in:foo` / `out:foo` / `animate:foo` shorthand or directive
  NAMES recorded as implicit identifier reads.
- `{#snippet name(params)}` parameters declared as template-
  scope bindings visible to the body.
- `<!-- svelte-ignore CODE -->` immediately before the instance
  `<script>` now bridges into the script's ignore stack.
  `svelte-ignore` inside parenthesized expressions also
  propagates.
- Runes-mode inference requires the rune name to be followed by
  `(` (optionally through `.word` for `$state.raw` etc.) so
  Svelte-4 `$$props.class` no longer flips files to runes mode.
- `element_implicitly_closed` no longer unwinds the regular-
  element stack on component-close tags
  (`<Foo><div/></Foo>` no longer fires).
- `AriaType::Boolean` validator strict-checks on all boolean
  aria attributes including bare `aria-hidden`.
- `use:dndzone={{ x }}` and similar object-literal attribute
  values no longer parse-fail as `BlockStatement`; wrapped in
  parens to force expression parsing.

### Parser

Sections parser tracks tag-depth with mustache-aware skipping.
Nested `<script>` / `<style>` inside a template (e.g.
`<svelte:head>{#if}<script>…</script>{/if}</svelte:head>` for
analytics snippets) stays in the template as a regular element
instead of being extracted as the Svelte instance script. The
mustache skipper respects JS string / template-literal / line-
comment / block-comment structure so `<`, `>`, `{` inside
expressions don't desync depth.

### CLI

- `--svelte-warnings` defaults to `native`.
- Bridge survives Node's CJS-as-ESM import shape — falls back to
  `mod.default.compile` when Node returns only `{ default }`.
  Fixes `compile is not a function` on Node-only installs
  (issue #4).

### Tests

- 14 new tag-depth + mustache-scan tests
  (`crates/parser/src/sections.rs`) covering every edge shape
  surfaced during the parser iteration.
- 18 new regression tests
  (`crates/svn-lint/tests/rule_gate_regressions.rs`) for the
  scope-walker / version-gate / runes-inference fixes.
- `bench_snapshot_thresholds` pins each version gate against
  real bench versions.

### Bench state

13 of 16 bench workspaces at exact Svelte-side-warnings parity
(Svelte-4 control, Svelte-5 control, plus real-world CMS / chart /
component-lib / UI-lib / tabular-data / file-share / media / music-PWA /
desktop-app / reader / ML-playground workspaces). Remaining drift
is error-layer (TS emit-shape) only; focus list in local
`notes/OPEN.md`.

## [0.3.9]

Patch release: three parity fixes driven by a real-world user
report on a SvelteKit monorepo. Closes the
`use:enhance={({form,data,submit}) => …}` callback-typing miss
(TS2339 ×3 per site upstream, silent on ours), the
`{#if form?.success}{form.error}` narrowing miss, and — same
root cause — the paraglide `m['login.pin']` literal-key miss.
Parity on `our Svelte-5 control` goes from 2 errors to 8,
matching upstream `svelte-check --tsgo` exactly.

Full investigation trail in [`docs/parity-findings-2026-04-21.md`](https://github.com/harshmandan/svelte-check-native/blob/main/docs/parity-findings-2026-04-21.md).

### Fixed — `use:ACTION={PARAMS}` callback parameters no longer lose contextual typing

`use:enhance={({formData}) => …}` and every other user-defined
action directive emitted as a dead `let __svn_action_attrs_N: any =
{}` placeholder. The `PARAMS` expression was discarded, so
TypeScript never saw the callback and its parameter destructure was
never checked. Users writing `use:enhance={({form,data,submit}) => …}`
(confusing the action's `SubmitFunction` param shape with the
`$props()` destructure names) got zero diagnostics — upstream fires
3 TS2339 per site.

Emit now mirrors upstream svelte2tsx's `__sveltets_2_ensureAction`
shape with our `__svn_` namespace:

```ts
const __svn_action_0 = __svn_ensure_action(
  enhance(__svn_map_element_tag("form"), callback)
);
```

The inner `enhance(...)` is a real call, so TypeScript contextually
types the callback against `SubmitFunction`'s declared input shape
and fires TS2339 on every wrong destructure name. New shim helpers
`__svn_ensure_action` + `__svn_map_element_tag` in
`svelte_shims_core.d.ts`. Design fixture at
`design/action_directive/` (per CLAUDE.md rule #8) proves the shape
before Rust touched — Clean case 0 errors, Wrong case exactly 3
TS2339 at expected positions.

Commit `f91fa70`.

### Fixed — template `{EXPR}` interpolations now participate in type-checking

Plain `{EXPR}` interpolations were emitted as nothing. Our template
walker voided ROOT identifiers via `find_template_refs` to keep
TS6133 off our back, but the full expression never landed in the
overlay — TypeScript had no opportunity to check it against
enclosing control-flow narrowing, scope bindings, or literal-key
types.

Visible consequences users reported:

- `{#if form?.success}{form.error}{/if}` on a hand-typed
  discriminated union — upstream fires TS2339 on the wrong-branch
  `.error` access; we fired nothing.
- `{m['login.pin']()}` where `m` is paraglide's generated messages
  object — upstream fires TS7053 on the missing literal key; we
  fired nothing.

Emit each plain interpolation as an expression statement inside its
enclosing scope, prefixed with a sentinel comment the post-walk
scanner uses as a token-map anchor:

```ts
if (form?.success) {
  void [form?.success];
  /*svn_I*/ form.error;
}
```

`collect_interpolation_token_map` zips `/*svn_I*/` sentinels with
fragment-walk-order expression ranges and pushes a TokenMapEntry
per site so diagnostics surface at the user's `{EXPR}` position.
Paren-wrap protects against multi-clause expressions parsing as
statement heads. 110 existing emit-snapshot fixtures picked up
additive lines.

Note: SvelteKit's `ActionData` type (from generated `$types`) uses
`OptionalUnion<U>` which synthesizes `?: never` for every other
branch's keys — reading `form.error` under `{#if form?.success}`
returns `undefined` rather than firing TS2339. Upstream behaves
the same. Our fix correctly catches hand-typed discriminated
unions; Kit-standard `ActionData` reads as designed.

Commit `ae15e45`.

### Fixed — code-frame caret aligns under tab-indented source

Windows-reported cosmetic bug: tab-indented file fired
`bind:value={addAssemblyPrice}` TS2322 but the `^^^` caret rendered
several visual columns LEFT of the actual error (under
`type="number"` on the line above). Root cause was the caret line
being padded with literal spaces while the source line rendered
tabs verbatim — terminal expansion made the source wider than the
padding counted for.

Fix mirrors each whitespace character from the source line into
the caret prefix (tab→tab, anything else→space). Terminal applies
the same expansion to both lines so the caret lands under the
error regardless of the configured tab width. Extracted
`render_code_frame` as a pure helper with 4 regression tests.

Purely cosmetic — no change to diagnostic content, line:col
numbers, or machine/machine-verbose outputs (those don't render a
code frame).

Commit `fe12814`.

### Workflow — GitHub releases instead of manual tagging

`gh release create vX.Y.Z --latest=true` now owns tag creation —
no more separate `git tag` + `git push origin vX.Y.Z` steps.
Collapses to one command, keeps tag and release in sync by
construction (v0.3.8 drifted between them for a session until
force-reconciled). CLAUDE.md updated.

Commit `91f59d4`.

### Docs — parity-findings write-up

New `docs/` tree with an investigation log covering the three
fixes above, the explicit non-findings (ActionData OptionalUnion,
duplicate `</form>`), a SvelteKit typed-callback audit, and
parity numbers before/after.

Commit `ae7d365`.

## [0.3.8]

Patch release: Windows fixes plus two correctness bugs that affected
real-world monorepo and SvelteKit-Vite setups.

### Fixed — Windows workspace paths produced "0 errors in 0 files"

`std::fs::canonicalize` on Windows always returns the verbatim/UNC
form `\\?\D:\…`, even for plain drive paths. tsgo silently rejects a
workspace root passed in that form (it doesn't treat `\\?\D:\app` as
equivalent to `D:\app`), and our lexical glob matching — TS `include`
patterns use forward slashes — doesn't survive the prefix either.

Symptom seen in a user report: upstream `svelte-check` found 19
errors in 6 files on a SvelteKit project under `D:\…`; our tool
reported `0 errors and 0 warnings in 0 files` with no indication of
what went wrong. Banner gave it away — we printed
`\\?\D:\GitHub\…`, upstream printed `d:\GitHub\…`.

Swapped every runtime `canonicalize` call (9 sites across cli, core,
typecheck) for `dunce::canonicalize`, which returns the plain
`D:\…` form whenever it's representable. Test code unchanged.

Commit `7ce4d8e`.

### Fixed — JS runtime discovery ignored PATHEXT on Windows

The compiler-warning bridge's `which_in_path` did `dir.join(name)`
with bare `"node"` / `"bun"` and tested `.is_file()`. On Windows the
real filename is `node.exe` — a bare-name lookup never hits. The
bridge silently no-op'd on every Windows install even when
Node/Bun was on PATH, and users lost the `svelte/compiler`
diagnostic stream (`state_referenced_locally`,
`element_invalid_self_closing_tag`, accessibility warnings, and the
other dozens of compiler warnings).

Extended discovery to iterate PATHEXT suffixes after the bare-name
attempt. Refactored into a pure `which_in(path_var, pathext, name)`
helper so tests exercise the PATHEXT logic without mutating process
env. Bare-name lookup still wins on Unix and also when callers pass
`node.exe` directly.

Commit `f11711e`.

### Fixed — `vite/client` / `vitest/globals` silently dropped from overlay

`is_resolvable_types_entry` classified any unscoped `foo/bar` entry
as a relative filesystem path, tried to locate it on disk, failed,
and silently dropped the entry before tsgo saw it. Real-world
victims: `vite/client`, `vitest/globals`, `astro/client`,
`@sveltejs/kit/types`. Dropping them erased the ambient types those
packages provide (`import.meta.env`, CSS module side-effect imports,
Vitest globals) and produced spurious TS2304/TS2307 cascades against
user code that type-checks cleanly upstream.

Replaced the relative-path heuristic with a narrower test (entry
starts with `.` or `/`) plus a `split_package_entry` helper that
separates the package root from the subpath. Package-subpath entries
are kept when `node_modules/<pkg>/package.json` exists; tsgo's own
module resolver handles the subpath via the package's exports map /
typesVersions / bundled `.d.ts` — trying to second-guess which file
it resolves to is what got us dropping valid entries in the first
place.

Commit `f563021`.

### Fixed — solution-style tsconfig redirect missed real monorepo layouts

`escape_solution_tsconfig` is the hatch that redirects from a
solution-style root tsconfig (pure references, no `include`) to a
sub-project tsconfig that actually declares path aliases, so `$lib`
and friends resolve. Two holes in the classifier:

1. References pointing at a variant filename like
   `tsconfig.app.json` were resolved to the directory and then
   rejoined with `tsconfig.json` — discarding the explicit filename
   the user wrote. Depending on the layout, the redirect either
   landed on a different file or bailed out entirely.

2. The `paths`-presence check parsed the leaf config alone and
   missed the common monorepo pattern where `paths` is declared
   once in a shared `tsconfig.base.json` and inherited via
   `extends`. We'd see an empty `paths` on the leaf, skip the
   redirect, and leave the user stuck on the solution root.

Honor the reference's filename when it names a file. Walk the full
extends chain via `load_chain` for the paths-presence check. Both
regressions are locked by dedicated unit tests.

Commit `a7d6adb`.

### Fixed — pnpm/bun tsgo discovery sorted versions lexicographically

`find_in_package_store` picked the "newest" `@typescript+native-preview@<version>`
directory by string order and took the last. String compare
mis-orders every multi-digit boundary: `...9` beats `...10`, `9.0.0`
beats `10.0.0`, `1.9.0` beats `1.10.0`.

Current tsgo naming `7.0.0-dev.YYYYMMDD.N` is fine today because `N`
stays single-digit and the date is fixed-width, but each axis is one
cycle from silently downgrading users. Replaced with proper semver
compare via the `semver` crate.

Commit `507a08f`.

### Cleanup — removed dead `svn-lint` crate

`crates/lint` was five lines of module docs with zero code. The CLI
listed it in its dependencies, pulling it through the workspace
build for nothing. Deleted. We'll re-add when there's an actual
rule to ship.

Commit `28ba5fd`.

## [0.3.7]

Patch release: two correctness fixes that close ~30% of the a CMS bench
parity gap with upstream `svelte-check` (default mode). No emit-
shape surprises; no regression on any other bench.

### Fixed — component-instantiation scaffolding for dotted tag names

`<UI.TextInput>`, `<ui.MyButton>`, `<Foo.Bar>` and similar member-
expression component invocations were silently disqualified in the
analyze phase — the template-check body emitted nothing for them.
Consequence: any type mismatch on props or bindings at those sites
passed silently.

The disqualifier was a one-line early return (`if c.name.contains('.')
return`) carried over from a v0.1 scope cut. The emit path already
handled dotted names correctly: `__svn_ensure_component(UI.TextInput)`
is a valid TypeScript member-expression value, and the root
identifier (`UI`) is voided via the existing template-refs pass so
the barrel import doesn't trip TS6133.

Dropping the return unlocks the full component-check emission for
member-expression components. a CMS bench (which leans heavily on this
pattern in its `UI` barrel) picked up 20 real user-code bugs that
were previously invisible.

Snapshot `htmlx2jsx/component-name-dot` updated to the new emit
shape; upstream svelte2tsx's reference output for the same input
produces the equivalent construct call.

Commit `a654def`.

### Fixed — TS5097 parity on user-authored `.ts`-extension imports

The overlay builder had `allowImportingTsExtensions: true` hardcoded
on every run — carried from an earlier architecture where we briefly
rewrote `.svelte` imports to `.svelte.ts`. That rewrite was removed
long ago; the flag stayed. Side effect: when users wrote
`import { x } from './helper.ts'` in their own Svelte source
(explicit `.ts` extension), our overlay silenced the TS5097 upstream
fires by default.

Upstream's own overlay doesn't set the flag. Their overlay inherits
whatever the user's tsconfig declares. If the user opts into
`allowImportingTsExtensions` in their tsconfig, `.ts`-extension
imports are fine; otherwise TS5097 fires.

This release matches upstream's behavior exactly. The flag is now
inherited, not forced. Users who want `.ts`-extension imports set
it in their own tsconfig and our overlay picks it up through the
extends chain.

The flag was NOT load-bearing on our `.svelte` overlay resolution:

- `allowArbitraryExtensions: true` handles `.svelte` imports via
  the `.d.svelte.ts` ambient sidecar.
- The sidecar's `.ts` re-export is legal under declaration-file
  rules regardless of `allowImportingTsExtensions`.

Both of those still work after the change.

Fixture `43-user-include-patterns` updated: the test's deliberate
`import from './helper.ts'` line now expects TS5097 (matching
upstream behavior for a user whose tsconfig doesn't opt in). The
fixture's note explains the parity rule.

Commit `b912b77`.

### Scoreboard delta on bench/a CMS bench

| metric                     | pre-release | post-`a654def` |          post-`b912b77` |
| -------------------------- | ----------: | -------------: | ----------------------: |
| ours errors                |         321 |            340 |                 **384** |
| overlap with upstream      |         156 |            159 |                 **219** |
| upstream-only (our misses) |         176 |            173 |                 **113** |
| files_with_problems        |          64 |             73 | **115** (upstream: 116) |

Net: **63 upstream catches newly matched this release** (from 176
misses down to 113, or +60 on the overlap axis). No regressions on
the other four benches (our Svelte-4 control 1124/0/2/2,
our Svelte-5 control 1/1/0/1, a music-PWA bench 88/0/0/0, a component-lib bench
832/8/127/51 all unchanged).

### Not in this release (documented for context)

- **Template-attribute-expression preservation for DOM elements.**
  Investigated as a candidate for closing the remaining 113 a CMS bench
  misses. Source-location classification showed the reachable yield
  was only ~20-35 catches (18-30%), not the ~80 originally estimated:
  ~55 of the 113 misses are in component-callback contexts we
  ALREADY emit, where the blocker is tsgo's current limitations on
  JSDoc typedef inference and discriminated-union narrowing — not
  anything we can fix in emission. Deferred until tsgo matures or a
  clearer ROI appears. See `notes/NEXT.md` session notes for the
  full classification.

## [0.3.6]

Patch release: sibling-visibility fix for solution-style monorepos

- pnpm isolated-install tsgo discovery + docs-URL routing accuracy.

### Fixed — sibling-visibility on solution-style tsconfig redirects

When the CLI detected a project-references solution root
(`{"files": [], "references": [...]}`) and redirected the workspace
into a sub-project (typical SvelteKit monorepo shape), transitive
imports into sibling referenced projects fired tsgo's "File not
listed within the file list of project" error. The sub-project's
`include` patterns only covered its own tree; imports into
`../services/...` or `../packages/...` had no matching glob in the
overlay.

The overlay builder now consults a new helper,
`svn-core::tsconfig::flatten_references_from_chain`, which:

- Walks the redirect target's full extends chain and collects every
  `references[]` entry across the chain.
- **Transitively expands** each ref's own `references[]` via BFS
  with visited-set dedupe (capped at 256 hops). Critical for
  monorepos where `packages/types` references `packages/db`, and the
  sub-project imports from types but pulls db files transitively.
- Each resolved reference contributes its own `include` (anchored
  at that project's dir) + `exclude` (absolute-anchored) + `paths`
  (BFS per-pattern first-wins).

The overlay merges these into its own include/exclude/paths, with
the redirect target's declarations winning per-key for paths
conflicts (inner-wins). The solution root's own `references[]` is
_not_ walked — following coordinating-only refs (e.g. `functions`
alongside `console`) would pull in code the user didn't ask us to
type-check.

Concrete result on a big monorepo: solution-root redirect
now completes without the init.ts "not listed" error. For
`bench/our Svelte-5 control` specifically: 2 errors → 1 error (the
remaining one is a legit tsgo "Excessive stack depth" on a heavy
conditional type).

Commits: `bad54b4` (canonical-loader regression fixtures lock the
groundwork), `33ebd89` (the helper + 5 unit tests), `abc2b11` (the
overlay wiring + 2 integration tests + CacheLayout plumbing).

### Fixed — tsgo discovery under pnpm/bun isolated installs

Previously only walked the hoisted path
`node_modules/@typescript/native-preview/bin/tsgo.js`. Under pnpm
8+'s default `shamefully-hoist=false` (and bun's analogous `.bun/`
layout), that symlink is absent — tsgo lives in
`node_modules/.pnpm/@typescript+native-preview@<version>/...`
instead. Users had to set `TSGO_BIN` manually.

New resolution order at each ancestor: env var → hoisted native →
hoisted wrapper → `.pnpm/` store walk (newest version wins,
native-binary preferred) → `.bun/` store walk. Hoisted paths still
beat store-fallback, so happy-path users pay zero extra cost.

Commit `662b207`.

### Fixed — compiler-error docs URL routing

Bridge-reported diagnostics with `Severity::Error` (parse errors as
`compile_error`, forced-error overrides) were being given the
`svelte.dev/docs/svelte/compiler-warnings#<code>` URL — the
warnings-page anchor, which 404s for error codes. Upstream routes
those to `compiler-errors#<code>`.

New `compiler_code_docs_url(code, severity)` helper matches
upstream `svelte-check`'s `getCodeDescription` shape exactly:
route by severity, require first-char-lowercase + `_`-or-`-`
separator (filters out TS numeric codes, PascalCase identifiers,
opaque slugs), normalize `-` → `_` before joining with the URL.
7 unit tests.

Commit `a2b0fdf`.

### Scoreboard

| bench                             | before (0.3.5) | after (0.3.6) | vs upstream --tsgo                                                                                                                                                                                                                           |
| --------------------------------- | -------------- | ------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| our Svelte-4 control              | 1124/0/2/2     | 1124/0/2/2    | exact parity                                                                                                                                                                                                                                 |
| our Svelte-5 control (redirected) | 2/2/0/2        | **1/1/0/1**   | **exact parity**                                                                                                                                                                                                                             |
| a component-lib bench             | 832/8/127/51   | 832/8/127/51  | +8 errors vs tsgo's 0, but upstream's tsgo run fatally bails on a missing `@types/node` (our overlay filters unresolvable types entries); upstream returns 0 from silent failure. Our 8 errors are real user-code bugs. We are more correct. |

## [0.3.5]

Patch release: two user-reported bugfixes + four code-review follow-ups.
Scoreboard unchanged from 0.3.0 (4/6 benches at exact `--tsgo` parity).

### Fixed — two user-reported bugs on real SvelteKit projects

- **Line numbers for diagnostics on hoisted imports were off by the
  count of synthetic `declare const` stubs.** When the emit crate
  prepends body-local stubs (`declare const <name>: ...;`) to the
  hoisted prelude, those lines have no entry in
  `hoisted_byte_offsets` — but the line-map walker was pairing each
  newline in `s.hoisted` with the next source offset anyway, shifting
  every real hoist's mapping N-stubs too far. Result: a TS6133 on a
  `type AppVideo` import pointed at the `}` of `interface Props` five
  lines below. Script-split now exposes `stub_prefix_len`; emit skips
  past the stubs before aligning with the first real offset. Root
  cause fix, no snapshot changes.
- **Type-only imports consumed only in template casts (`{fn(x as
AppVideo)}`) fired false-positive TS6133.** `AppVideo` is
  type-only, so `collect_top_level_bindings` correctly skipped it
  (voiding a type name fires TS2693), which meant template-ref
  intersection never matched it, which meant the void-refs block
  never referenced it — tsgo then flagged the import as unused. Fix:
  emit now intersects `find_template_refs` output with a new
  `collect_type_only_import_bindings` set and synthesizes
  `type __svn_tpl_type_refs = [AppVideo, …];` at module scope.
  Locked with bug fixture
  `60-type-only-import-used-in-template-cast`.

### Changed — code-review follow-ups

- **UTF-8-safe ANSI stripping in the tsgo output parser.** The
  byte-by-byte cast to `char` turned UTF-8 continuation bytes into
  U+0080–U+00FF individually, corrupting any Unicode filename or
  diagnostic message before the line-map and path-reverse stages
  saw them. Non-ESC runs are now copied as string slices; CSI
  introducer bytes and terminators are all ASCII so byte-indexed
  lookaheads still land on char boundaries.
- **Overlay builder now walks the canonical tsconfig loader.** The
  overlay had its own extends walker with a local `resolve_extends`
  that only did `dir.join(reference)` — missing package extends
  (`@tsconfig/svelte`), `.json` inference, node_modules walk-up,
  and implicit `${configDir}` substitution. Each of the five
  derived fields (`paths`, `rootDirs`, `include`, `exclude`,
  `types`) also re-read every tsconfig in the chain. A new
  `svn-core::tsconfig::load_chain` helper returns every visited,
  substituted config in BFS order; the overlay walks it once and
  aggregates per-field. Reinstates CLAUDE.md's "no parallel
  JSON-reading shortcuts" rule. Drops `json5` from `svn-typecheck`.
- **Instance script parsed once per document in the emit hot
  path.** `emit_document` was calling `parse_script_body` on the
  same `instance.content` in four separate places with four
  separate allocators. Consolidated to one top-of-function parse;
  every downstream analyze call reads from the single
  `&parsed.program`. Props-type annotation also cached once. Pure
  perf win — no semantic change, all snapshots unchanged.
- **Store auto-subscribe scanner skips strings, comments, and
  template-literal static segments.** Previously a `"$store"`
  inside a string literal was accepted as a potential store ref
  (documented limitation in a test). Mini lexer now skips
  `// line`, `/* block */`, `"…"`, `'…'`, and the static parts of
  template literals. `${…}` interpolations are re-scanned as code
  so a `$store` inside an interpolation still gets picked up. Brace
  counting per-level handles nested object literals and nested
  templates. Escape-aware.

### Scoreboard

Unchanged from 0.3.0. Warmed bench on a component-lib bench: 832/8/127/51,
matching pre-session.

## [0.3.0]

### Parity milestone

**4 of 6 real-world benches at exact parity with `svelte-check --tsgo`.**

| bench                                     | ours (F/E/W/P)   | svelte-check --tsgo | svelte-check default | Δ E                    |
| ----------------------------------------- | ---------------- | ------------------- | -------------------- | ---------------------- |
| our Svelte-4 control (1000-file monorepo) | 1124/**0**/2/2   | 1125/1/2/3          | **6511/0/2/2**       | **0** ✓                |
| our Svelte-5 control                      | 1359/**2**/44/17 | 1359/**2**/44/17    | 7290/1/44/16         | **0** ✓                |
| a music-PWA bench                         | 88/**0**/0/0     | 88/**0**/0/0        | 1410/0/0/0           | **0** ✓                |
| a reader bench/web                        | 113/**0**/0/0    | 113/**0**/0/0       | 724/0/0/0            | **0** ✓                |
| a CMS bench                               | 211/321/67/64    | 211/419/67/121      | 5501/331/67/116      | −10 vs default         |
| a component-lib bench                     | 832/8/127/51     | 750/0/127/48        | 5751/6/127/49        | +8 (ours more correct) |

### Added — component-prop + event typing

- **Required-prop enforcement direct from the `new $$_C(...)` site.**
  `__svn_ensure_component`'s `Component<P>` overload returns
  `new (options: { target?: any; props?: P }) => ...` — exact `P`
  (no Partial wrapping). Missing required props fire TS2741 at
  the component-name source position with the precise error code
  (Svelte 5 runes). Earlier v0.3 attempts used a
  `satisfies InstanceType<...>['$$prop_def']` trailer that fired
  TS1360 at wrong position; replaced by direct prop-type check +
  call-span `TokenMapEntry` source-map.
- **Typed event-handler narrowing via `$$Events`.** When a child
  component declares `interface $$Events` or `type $$Events`, the
  consumer's `<Child on:myevent={handler}>` narrows `handler`'s
  parameter to `(e: CustomEvent<E[K]>) => any`. Wrong-payload
  handlers fire TS2345 at the `on:event={handler}` directive
  position. Typed `createEventDispatcher<T>()` WITHOUT `$$Events`
  stays lax (matches upstream's `__sveltets_2_with_any_event`
  behavior — see DEFERRED notes).
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
- **`bind:this` on DOM elements.** Direct-assignment emit
  (`EXPR = null as any as HTMLElementTagNameMap['tag'];`) —
  not lambda-wrapped. TypeScript's control-flow analysis
  narrows `EXPR` from `T | null` to `T` for subsequent uses,
  matching upstream's `Binding.ts:86-93` semantics. Lets
  `let ref = $state<HTMLElement | null>(null)` flow as
  `HTMLElement` at downstream prop-passing sites.
- **`bind:this` on components** covers both simple-identifier
  and member-expression targets (`bind:this={refs.input}`).
- **`bind:X={getter, setter}` get/set form.** Svelte 5's
  two-function bind syntax modeled as `PropShape::GetSetBinding`
  emitting `name: (getter)()` so required props are seen present.
- **Byte-span source maps.** New `TokenMapEntry` alongside
  `LineMapEntry` in the typecheck mapper. Diagnostics in
  synthesized regions get remapped to tightest-matching source
  byte spans; drops only when neither token-map nor line-map
  covers. Per-feature post-scans for component calls,
  DOM-binding checks, `bind:this` assignments, `$on` event-name
  literals pair overlay sites 1:1 with user source.

### Added — shape alignment with upstream

- **Conditional Svelte-4 widen** matching upstream's
  `__sveltets_2_PropsWithChildren<Props, Slots>` +
  `__sveltets_2_with_any(…)` factory pattern:
  - No widen for pure-Svelte-5-runes components.
  - `& { children?: any }` when the component has `<slot>` usage.
  - `& { [index: string]: any }` (`__SvnAllProps`) when the
    component uses `$$props` / `$$restProps` (whole-document
    scan covers both script AND template `{...$$props}` spreads).
  - Both together when both apply.
    Previous version intersected `{slot?, class?, style?} &
{[index: string]: any}` unconditionally, contaminating tsgo's
    assignability check (missing-required-prop fired TS2322 top-
    level with TS2741 as sub-message instead of TS2741 directly).
    Now matches upstream's minimal widen pattern; TS2741 surfaces
    with the precise error code (Svelte 5 runes) or as upstream's
    TS2322+TS2741 chain (Svelte 4 widen cases — same underlying
    info, same visual position).

### Added — diagnostic method + fixture infrastructure

- **"Diff the real upstream artifact" method** codified in
  CLAUDE.md. Concrete 6-step process: anchor on a real failing
  file → read upstream's actual svelte2tsx output → read ours via
  `--emit-ts` → side-by-side diff at the diagnostic site → lock
  upstream's shape as a tsgo fixture → port. Used to close every
  bench parity delta this release.
- **Phase A fixture gate** (`design/phase_align_upstream/`).
  Six tsgo-validated TS files that anchor upstream's
  `Render<T>` + `isomorphic_component` pattern for Svelte 5
  runes components, typed dispatchers, and generic-bearing
  dispatchers. Regression anchor for future shape work.

### Added — bench tooling

- **`scripts/bench.mjs --mode parity`.** Runs our binary +
  upstream `svelte-check` + `svelte-check --tsgo` (when
  available) on the same workspace; prints a side-by-side
  comparison of FILES, ERRORS, WARNINGS, FILES_WITH_PROBLEMS
  counts. Exits non-zero on drift from the best-available
  upstream baseline. Solution-style tsconfig redirects detected
  - propagated. `findUpstreamSvelteCheck` walks
    `node_modules/.bin/`, `node_modules/.pnpm/`,
    `node_modules/.bun/svelte-check@X/`, and
    `node_modules/.pnpm/svelte-check@X/` layouts; prefers 4.4+
    (first `--tsgo`-capable release) when multiple candidates
    exist.

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
  ` ```text ` fenced block; rustdoc no longer tries to
  compile it as Rust.

### Fixed — corner cases

- **`bind:NAME` shorthand on components.** Bare-shorthand form
  (`<Child bind:items />` desugaring to `<Child
bind:items={items}>`) now emits as `PropShape::Shorthand`
  rather than being silently dropped.
- **`bind:X={getter, setter}` consumer-side.** Svelte 5's
  two-function bind form is now modeled; consumers of children
  with required props that the user passes via get/set no
  longer fire spurious "missing required prop" TS2741.
- **DOM-binding flow narrowing.** One-way DOM bindings
  (`bind:clientWidth`, `bind:contentRect`, etc.) previously wrapped
  the assignment in a never-called lambda — isolating the
  assignment from TS flow analysis. Now emits as a plain
  statement so the assignment's RHS type narrows EXPR for
  subsequent uses, matching upstream's `Binding.ts:86-93`
  semantics. Eliminates `our Svelte-4 control`'s last FP (1→0 errors).
- **`bind:this` on DOM elements: same narrowing fix.** Previously
  lambda-wrapped; now plain assignment. Closes the
  `let iconEl = $state<HTMLElement | null>(null)` →
  `<button bind:this={iconEl}>` → `<Child {iconEl}>` narrowing
  gap that caused a FP on our Svelte-5 control.

### Known limitations (match upstream behavior)

These are intentional gaps where upstream `svelte-check` is
deliberately permissive; our tool matches that laxity to
preserve drop-in parity. Every item below verified against
upstream source with file:line citations.

- **Typed `createEventDispatcher<T>()` consumer narrowing.**
  Child: `const d = createEventDispatcher<{change: {checked: boolean}}>()`,
  parent: `<Child on:change={({detail}) => ...}>` — `detail` is
  `any`, not narrowed. Upstream's `__sveltets_2_with_any_event`
  (svelte-shims.d.ts + addComponentExport.ts:417) deliberately
  widens consumer handlers unless `<script strictEvents>` or
  explicit `$$Events` is set. Our existing lax `$on` overload
  matches.
- **`bind:value` on `<select>`.** Accepts any type. Upstream's
  `svelte-jsx.d.ts:1342` declares `HTMLSelectAttributes['value']?:
any`. No `<option>` value-union inference anywhere upstream.
- **`bind:group` on `<input type="checkbox|radio">`.** Silent.
  Upstream's `Binding.ts:99-108` emits
  `EXPR = __sveltets_2_any(null);` (widen-to-any). Neither
  direction catches errors upstream either.
- **Cross-HTMLElement distinctions for `bind:this`.** TS's DOM lib
  treats element subtypes as structurally compatible. Upstream's
  `Binding.ts:71-94` accepts any HTMLElement subtype. Blocked on
  [TypeScript issue #45218](https://github.com/microsoft/TypeScript/issues/45218)
  (stale since 2021).

### Known bench discrepancies

- **Upstream default `svelte-check` reports 5-12× more FILES** on
  large workspaces (our Svelte-4 control: 6511 FILES vs ours 1124
  vs `svelte-check --tsgo` 1125). Upstream default crawls all
  `.svelte/.ts/.js/.d.ts` on disk via TypeScript's LanguageService
  (supporting hover/autocomplete paths), including declaration
  files unrelated to the type-check surface. Our FILES matches
  `svelte-check --tsgo` exactly on every verified bench (the
  meaningful "what was actually type-checked" count). Users
  wanting default-svelte-check denominator parity should run
  `--tsgo`.

### Performance

- our Svelte-4 control bench (1124 files): warm 2.31s median (v0.2.5
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
  trendline refactor) and 2 errors on `an ML-playground bench`.

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

| Tool                     | Warm     |
| ------------------------ | -------- |
| `svelte-check-native`    | **~3 s** |
| `svelte-check-rs`        | ~11 s    |
| `svelte-check --tsgo`    | ~13 s    |
| `svelte-check` (default) | ~40 s    |

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
