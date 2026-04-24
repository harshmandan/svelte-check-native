# TS-overlay component Props in Props slot — fixture

Locks fix #2 from the 2026-04-24 investigation write-up (private notes).

## The bug

TS-overlay default-export emits `Component<Record<string, any> &
__SvnAllProps, {precise prop shape}>` — the ACTUAL prop shape goes
in the Exports slot (2nd generic), not the Props slot (1st).
Consumer callsites passing arrow callbacks like `onclick={(e) =>
...}` lose the `MouseEvent` contextual type for `e`, firing TS7006
"Parameter 'e' implicitly has 'any' type".

## What upstream does

`language-tools/packages/svelte2tsx/svelte-shims-v4.d.ts:278-286`
defines `__sveltets_2_IsomorphicComponent<Props, Events, Slots,
Exports, Bindings>` — Props FIRST. svelte2tsx's `$$render()` returns
`{props: {...precise}, events: {...}, slots: {...}, exports: {...},
bindings: ""}`, wrapped by `__sveltets_2_isomorphic_component(
$$render())`. Props type flows to the constructor signature and
contextual typing works.

## Fix target shapes

Two equivalent shapes validated here — either is a viable port:

**Option A — straight swap.** Keep `Component<>` but put the real
shape in the Props slot:
```ts
declare const X: import('svelte').Component<
    { bar: Object; onclick?: (e: MouseEvent) => void; ... },  // <-- was Exports
    {}                                                         // <-- was loose record
>;
```

**Option B — `Awaited<ReturnType<$$render>>['props']` (matches
JS-overlay).** Makes `$$render()` return a typed object whose
`props` field carries the shape; extract it at module scope:
```ts
async function $$render() {
    // ... body ...
    return { props: { bar, onclick, ... }, events: {}, slots: {}, exports: {} };
}
type __Props = Awaited<ReturnType<typeof $$render>>['props'];
declare const X: import('svelte').Component<__Props, {}>;
```

## Why Option B is preferable for our port

- Our JS-overlay path already uses this shape (see
  `emit_render_body_return` + the `/** @typedef {Awaited<...>['props']} */`
  in `crates/emit/src/lib.rs`). The TS-overlay path should mirror
  it for consistency.
- The $$render function's body already has the typed locals we
  want in Props — no need to re-derive them at module scope, which
  is error-prone for generic components.
- Mirrors upstream's `__sveltets_2_isomorphic_component($$render())`
  shape at the value layer.

## Validation

```sh
cd design/ts_overlay_component_props/fixture

# Clean (Option A + Option B both compile without errors)
mv src/bar_overlay_broken.ts src/bar_overlay_broken.ts.hidden
tsgo --project tsconfig.json --noEmit --pretty false
# exit 0, no output

# With broken overlay: exactly 2 TS7006 on the arrow params
mv src/bar_overlay_broken.ts.hidden src/bar_overlay_broken.ts
tsgo --project tsconfig.json --noEmit --pretty false
# src/bar_overlay_broken.ts(37,23): error TS7006: Parameter 'e' implicitly has an 'any' type.
# src/bar_overlay_broken.ts(40,30): error TS7006: Parameter 'e' implicitly has an 'any' type.
```

## Files

- `src/svelte_shim.d.ts` — cut-down `svelte` ambient + our
  `__svn_ensure_component` helper overload chain (minimal subset
  needed for the consumer-site type check).
- `src/bar_overlay_broken.ts` — current emit shape. Reproduces
  TS7006.
- `src/bar_overlay_clean.ts` — Option A target shape. Clean.
- `src/bar_overlay_awaited.ts` — Option B target shape. Clean.

## Upstream reference

- `language-tools/packages/svelte2tsx/svelte-shims-v4.d.ts:278-286`
  — `IsomorphicComponent<Props, ...>` interface + factory.
- `language-tools/packages/svelte2tsx/test/svelte2tsx/samples/*/expectedv2.ts`
  — reference emit shapes.
- commit `9a093d5` — fixed the TS8010 abort that had been hiding
  the cluster-B TS7006s, which is why this bug took so long to
  surface.
