# JS-overlay emit shape (validated 2026-04-23)

Spec for the `crates/emit` JS-source branch — the second emit form
that runs when a `.svelte` source has no `<script lang="ts">`. The
overlay extension switches from `.svelte.svn.ts` to `.svelte.svn.js`
so tsgo applies JS-loose inference (`let x = $state([])` → `any[]`)
instead of TS-strict (`never[]`). This closes the
CodeMirror-wrapper cluster on a CMS-style bench (62 errors → 0)
and is expected to close hundreds across Svelte-4-heavy JS
workspaces.

## Why this exists

The /tmp/js_vs_ts/ minimal repro (preserved): same source code, `.js`
extension → 0 errors, `.ts` extension → 2 errors. Difference is
load-bearing — tsgo applies different inference rules per extension
under `checkJs: true` + `noImplicitAny: false` (a common Svelte-5
CMS-style tsconfig shape).

We currently always emit `.svn.ts`. JS-Svelte sources go through
TS-strict inference that the user never opted into. Mirroring
upstream's emit (which writes `.svelte.js` in
`++CodeMirror.svelte.js`) closes the gap.

## Rerun

```sh
TSGO=$(find . -name tsgo -path '*native-preview-darwin-arm64*' -type f | head -1)
cd design/js_overlay/fixture
"$TSGO" --pretty false -p tsconfig.json
```

Expected output is exactly two diagnostics, both in `Errors.svn.js`:

```
src/Errors.svn.js(10,1): error TS2322: Type 'number' is not assignable to type 'string'.
src/Errors.svn.js(17,13): error TS2345: Argument of type 'number' is not assignable to parameter of type 'string'.
exit=2
```

Any other diagnostic is a regression. Any missing diagnostic from
`Errors.svn.js` means the JS-overlay shape silently dropped a real
diagnostic — also a regression.

## Pattern table — what the emit branch must produce

Each fixture file under `fixture/src/<NN>_*.svelte.svn.js` is one
pattern. The hand-written model is the shape the Rust emit branch
must produce when `Document::script_lang() == Js`.

| Pattern | TS-overlay form | JS-overlay form | Fixture |
| :--- | :--- | :--- | :--- |
| `$state` inference | `let x: never[] = $state([])` (strict; cascade of TS2339) | `let x = $state([])` (any[] under JS-loose) | `01_state_inference.svelte.svn.js` |
| `$props` destructure | `let { v }: $$Props = $props()` | `/** @type {$$Props} */ let { v } = $props()` plus `@typedef` | `02_jsdoc_props.svelte.svn.js` |
| Default-export | `declare const __svn_component_default: import('svelte').Component<P>; export default __svn_component_default;` | `/** @type {import('svelte').Component<P>} */ const __svn_component_default = null; export default __svn_component_default;` | `03_default_export.svelte.svn.js` |
| Store auto-subscribe | `let $counter!: __SvnStoreValue<typeof counter>;` | `/** @type {__SvnStoreValue<typeof counter>} */ let $counter = /** @type {any} */ (null);` | `04_store_unwrap.svelte.svn.js` |
| `__svn_tpl_check` casts | `element = null as any as HTMLDivElement;` | `element = /** @type {HTMLDivElement} */ (/** @type {any} */ (null));` | `05_template_check.svelte.svn.js` |
| `$$render` wrapper | `async function $$render<T>() { ... }` | `async function $$render() { ... }` (no generics — JS source can't carry `<script generics>`) | `06_render_wrapper.svelte.svn.js` |

## Constraints discovered while authoring the fixture

- **`$state` shim must stay 2-overload** (`<T>(initial: T)` and `<T>()`) — adding `<T>(initial: null)` resolves T to `unknown`, which survives the truthy check and breaks `clearTimeout(t)`. Production svelte_shims_core.d.ts:264-265 already gets this right; the JS overlay relies on the same shape.
- **Triple-slash `<reference types>`** is the cleanest way to thread shims into a `.js` overlay — same convention used by the existing `design/phase_a/fixture/`.
- **Module-resolution leak via `import('svelte')`**: the design fixture stubs the `svelte` module (`src/svelte_shim.d.ts`); production overlays will resolve to the user's installed `node_modules/svelte/types/index.d.ts`.
- **`!:` definite-assign and `as any` are TS-only** — every site that emits them today needs a JSDoc-cast rewrite. Pattern enumeration above is exhaustive (audited against the production CodeMirror.svelte.svn.ts).

## Out of scope for this fixture

- The `__sveltets_2_*` family of helpers upstream uses (e.g. `__sveltets_2_isomorphic_component`, `__sveltets_2_with_any_event`). Our class-wrapper architecture replaces them; the JS overlay just needs to mirror our existing shape with TS-only syntax stripped, not adopt upstream's runtime helpers.
- Generics on `$$render` — JS-Svelte sources cannot carry `<script generics="X">` (Svelte-5 runes / TS-only feature). Emit can drop the generic-param block unconditionally for JS sources.

## Patterns uncovered by the upstream_sanity diff (2026-04-23)

Running `upstream_sanity` with the JS-overlay flag flipped surfaced
three Svelte-4 `export let` patterns that our TS-overlay path handles
by synthesizing TS syntax (`!:`, `: any`, `as T`) — none of those
survive a `.js` file; tsgo fires TS8010 and aborts semantic analysis
for the enclosing function, which silently swallows every other real
diagnostic. Upstream svelte2tsx produces JS-safe equivalents for each.

Each pattern is anchored on a representative source file from
`language-tools/packages/svelte-check/test-error/` and validated via
`node /tmp/s2tsx.mjs <file>` against the installed svelte2tsx (the
same `svelte2tsx(..., { isTsFile: false, mode: 'ts', emitJsDoc: true })`
call upstream `svelte-check` makes in `incremental.ts:244-256`).

### 1. Definite-assign via `__sveltets_2_any(name)` self-assignment

**Source:** `Jsdoc.svelte` — `/** @type {boolean}*/ export let b; b;`
**Upstream emit:** `let b; b = __sveltets_2_any(b); /* */; b;`
**Upstream source:** `svelte2tsx/src/svelte2tsx/nodes/ExportedNames.ts`
`sanitize`/`prependStr` path — emits a separate reassignment line
after the declaration.

Our current behaviour: `rewrite_definite_assignment_in_place` splices
`!` after the name (TS-only). In JS we need a separate fn that
appends `NAME = __svn_any(NAME);` after the declaration terminator.
Shim already has `__svn_any<T = any>(): T`; add an overload
`declare function __svn_any(x: any): any;` to cover the self-assign
pattern.

### 2. Props return carrying JSDoc-typed per-key shape

**Source:** `Jsdoc.svelte` — `/** @type {boolean}*/ export let b;` with
consumer `<Jsdoc />` (no props).
**Upstream emit:** `return { props: { /** @type {boolean}*/ b: b } };`
**Upstream source:** `svelte2tsx/src/svelte2tsx/nodes/ExportedNames.ts`
`createPropsStr` — emits each exported local as an object property
with a leading JSDoc `@type` comment carrying the declared type.

Our current behaviour: `emit_render_body_return` JS path emits
`return { props: /** @type {any} */({}) };`. The `any` erases the
required-prop signal → consumer's `<Jsdoc />` misses TS2741. Fix:
when `PropsInfo.source == SynthesisedFromExports`, embed the
synthesised `type_text` as the props cast instead of `any`.

### 3. Bind target type cast via `null as T` in ignore-comment pair

**Source:** `Jsdoc.svelte` — `<div bind:contentRect={rect}>`
**Upstream emit:** `rect = /*Ωignore_startΩ*/null as DOMRectReadOnly/*Ωignore_endΩ*/;`
**Upstream source:** `svelte2tsx/src/htmlxtojsx_v2/nodes/Binding.ts`
— emits the TS cast wrapped in `/*Ωignore_startΩ*/…/*Ωignore_endΩ*/`
markers that tsgo treats as ignored ranges.

Our current behaviour: `rect = __svn_any(null);` — widens to any,
loses the bind-target type. Fix: use `__svn_any_as<T>(rect);` helper
already in the shim (line 527) to bind the target's declared type to
`T` without mutating flow. Emission site: `emit_dom_binding_checks_inline`.

## Implementation status

**Default-on as of <date of flip commit>.** The `SVN_JS_OVERLAY=1` gate
was removed after:

- Bench A/B: CMS bench gap 62 → 24; charting-lib bench 164 → 0;
  Svelte-4/5 controls and component-lib bench held at 0.
- CodeMirror.svelte diagnostics: 62 → 37, byte-identical to upstream.
- `upstream_sanity` fixtures: clean project 0 errors; error project
  fires the expected 7 after patterns 1-3 above landed.
- Design fixture under `design/js_overlay/fixture/` still produces
  exactly the 2 expected diagnostics.

CLI dispatches per-file on `doc.script_lang() == Ts`, mirroring
upstream svelte-check's `isTsSvelte(text)` dispatch in
`language-tools/packages/svelte-check/src/incremental.ts:213`.

Rough Rust call-sites that branch on is_ts:

- `lib.rs::apply_script_body_rewrites` — skips `widen_untyped_exported_props_in_place`
  and `rewrite_definite_assignment_in_place` when JS; calls
  `rewrite_definite_assignment_jsdoc_in_place` instead.
- `lib.rs::emit_render_body_return` — JS path emits
  `/** @type {{<exports shape>}} */({})` when PropsInfo is
  SynthesisedFromExports.
- `lib.rs::emit_default_export_declarations` — JS path emits JSDoc
  `@type` form on a runtime `const`.
- `lib.rs::emit_dom_binding_checks_inline` — JS path uses
  `__svn_any_as<T>(target);` for bind: casts.
- `script_split.rs` — strip `: T` annotations and `!:` definite-assign on synthesized lets.

Plus cache change: `CacheLayout::generated_path_with_lang(source, is_ts)` —
picks `.svelte.svn.js` when `is_ts=false`.
