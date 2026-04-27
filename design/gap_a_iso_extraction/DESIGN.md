# Gap A — IsomorphicComponent extraction (validated 2026-04-27)

## Problem

`threlte/packages/extras` over-fires +2 errors on
`InstancedMeshes.svelte:21,31` — the `(typeof Instance)[]` /
`Parameters<typeof Instance>` patterns common in Threlte's
instancing code.

Diagnostic (ours):
> Type `((internal: unknown, props: InstanceProps) => { $set?: any;
> $on?: any; })[]` is not assignable to type `$$IsomorphicComponent[]`.

Upstream is clean.

## Root cause

Our `default_export.rs` ALWAYS emits a per-component
`$$IsomorphicComponent` interface for TS-overlay components. The
interface has both a `new(...)` ctor signature AND a `(...)` callable
signature. When user code does `Parameters<typeof Comp>` the inner
arrow type ends up being just `(internal, props) => Exports` — a
plain callable that **cannot satisfy** the `new(...)` requirement of
the iso interface.

Upstream's strategy is more nuanced — `addComponentExport.ts` selects
between three shapes:

| Component profile | Upstream emit | Has `new`? | `$set`/`$on`? |
| :--- | :--- | :--- | :--- |
| Has generics | per-component `$$IsomorphicComponent` | yes | no |
| Runes + no slots + no events | `__sveltets_2_fn_component` returning `Component<P, X, B>` | **no** | yes (in `Component`) |
| Otherwise | shim `__sveltets_2_IsomorphicComponent` | yes | yes |

For Threlte's `Instance.svelte` (runes, no slots, no events), upstream
selects the **second** path: `Component<P, X, B>`. `Component<>` has
**only a call signature** — no `new(...)` ctor. So
`Parameters<typeof Instance>` extracts cleanly, and the inner arrow
satisfies `Component<>` structurally.

We unified everything into the per-component iso, missing this case.

## Validated fix shape

Run `tsgo --pretty false -p tsconfig.json` from this fixture:

- `consumer_threlte.ts:18,32` — both iso variants (ours WITH
  `& { $set?, $on? }`, upstream-iso WITHOUT) FAIL the `(typeof X)[]`
  pattern. Confirms the iso `new(...)` ctor is the obstacle, not the
  `& { $set?, $on? }` intersection.
- `consumer_threlte.ts:46` — `Component<P, X, B>` shape PASSES. This
  is the path forward.
- `consumer_component_target.ts` — `let X: Component<{}> = NoProps;`
  passes for ALL three source shapes when the source has empty Props.
  So switching no-slot/no-event runes components from iso to
  `Component<>` does NOT regress the v0.5+1 layerchart case.

## Implementation plan

In `crates/emit/src/default_export.rs`, branch the TS-overlay default
export path. When ALL of:

1. `isSvelte5` (we always assume this in TS overlay)
2. runes mode (script uses `$props()`, `$state()`, `$derived()`, etc.)
3. no `<slot>` elements, no `let:` consumer directives
4. no `interface $$Events` / `$$Events` type / `createEventDispatcher` /
   `dispatch` calls

emit:

```ts
declare function __svn_fn_component<
    Props extends Record<string, any>,
    Exports extends Record<string, any>,
    Bindings extends string
>(klass: { props: Props; exports?: Exports; bindings?: Bindings }):
    import('svelte').Component<Props, Exports, Bindings>;

const __svn_component_default = __svn_fn_component($$render());
type __svn_component_default = ReturnType<typeof __svn_component_default>;
export default __svn_component_default;
```

(Helper goes in `crates/typecheck/src/svelte_shims_core.d.ts` once,
not inlined per-file.)

Otherwise: keep the current `$$IsomorphicComponent` interface emit
unchanged. Generic + slot + event cases still need it.

## Where the conditions are tracked today

- `summary.uses_slot` — already tracked in `TemplateSummary` for
  legacy `<slot>` detection
- `summary.uses_let_directive` — slot-let tracking
- `props_info.has_event_dispatcher` — `createEventDispatcher` calls
- `props_info.dollar_dollar_events` — `$$Events` interface/type
- `props_info.uses_runes` — already discriminates Svelte-4 vs
  Svelte-5 runes mode

## Out of scope

- Generic components retain per-component `$$IsomorphicComponent`.
  No change for them; the Threlte `InstancedMeshes` cascade goes away
  because `Instance` (the inner type) becomes Component-shaped, and
  `(typeof Instance)[]` is then satisfiable.
- Svelte-4 callable shape, JS overlay default export: unchanged.

## Risk

- ~50-100 emit_snapshots will need re-baselining. The new shape is
  visibly different (uses `__svn_fn_component(...)` instead of
  inline interface). Snapshot review on a stratified sample catches
  regressions.
- Any overlay-shape-dependent bug fixture that hand-asserts the iso
  shape needs to flip to the Component-shape assertion.

## Validation gates

1. tsgo on this fixture: 4 errors (the 2 broken iso variants, no
   diagnostics on the Component variant or the bare-Component target).
2. After implementation: re-run threlte/extras — expect +2 errors
   delta closes to 0.
3. After implementation: re-run all existing benches — expect no
   regressions on the 12 currently byte-perfect.

## Cross-checks done

- bench/threlte/packages/extras: real failing file
  `Instancing/InstancedMeshes/InstancedMeshes.svelte:21,31`
- upstream emit dump confirms Instance.svelte uses
  `__sveltets_2_fn_component` not iso
- `addComponentExport.ts:343` confirms the selection condition
- `svelte-shims-v4.d.ts:273-276` declares `__sveltets_2_fn_component`
  return type as `Component<P, X, B>`
- `svelte/types/index.d.ts:141-172` declares Svelte's `Component`
  interface (callable only — no `new` ctor)
