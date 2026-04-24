# Phase 2 fixture-first: class-wrapper pattern

CLAUDE.md architecture rule #8: every new emit shape is
tsgo-validated on a hand-written fixture before Rust implementation
begins. This directory holds the fixtures gating Phase 2
(R1 — class-wrapper pattern, per `notes/PLAN.md`).

## The problem we're solving

**Real-world repro:** a charting-lib `BarChart.svelte` component under `src/lib/components/charts/`.

The user's component declares `interface $$Props` with properties
typed as `typeof <body-local>`:

```svelte
<script lang="ts" generics="TData">
  interface $$Props {
    handler?: typeof handler;
    labels?: typeof labels;
  }

  export let handler: (item: TData) => void;
  export let labels: ChartLabels<TData> | boolean = false;
  …
</script>
```

`handler`, `labels`, … are body-local `export let` declarations;
their types reference the script's generic `TData`. The `$$Props`
interface that describes the component's props bag references these
body-locals via `typeof`.

**Current emit shape** (from `target/release/svelte-check-native --emit-ts`,
captured 2026-04-22):

- `$$Props` is declared *inside* `async function $$render_<hash><TData>() { ... }`,
  so `typeof handler` resolves inside the render function's scope —
  fine for the render body itself.
- The module-scope default-export declaration
  (`declare const __svn_component_default: <TData>(__anchor, props: Partial<{...}>) => any`)
  can't mention `$$Props` at module scope without firing TS2304
  ("Cannot find name 'handler'" etc). So we inline-expand the props
  type with `any` fallbacks in place of the body-local refs.
- Consequence: consumers like
  `<BarChart handler={(val) => …} />` see the prop context as `any`,
  TS doesn't contextually-type the arrow's `val`, TS7006 fires.

**Upstream svelte2tsx's shape** (from `node_modules/svelte2tsx/index.mjs`
run directly on BarChart.svelte, captured 2026-04-22):

```ts
function $$render<TData>() {
    interface $$Props extends ChartProps { … }
    // ... body ...
    return { props: { …body-local-typed fields… } as $$Props, events: {}, slots: {…} };
}

class __sveltets_Render<TData> {
    props()    { return $$render<TData>().props; }
    events()   { return __sveltets_2_with_any_event($$render<TData>()).events; }
    slots()    { return $$render<TData>().slots; }
    bindings() { return ""; }
    exports()  { return {}; }
}

interface $$IsomorphicComponent {
    new <TData>(options: import('svelte').ComponentConstructorOptions<
        ReturnType<__sveltets_Render<TData>['props']> & { children?: any }
    >): import('svelte').SvelteComponent<
        ReturnType<__sveltets_Render<TData>['props']>,
        ReturnType<__sveltets_Render<TData>['events']>,
        ReturnType<__sveltets_Render<TData>['slots']>
    >;
    <TData>(internal: unknown, props: ReturnType<__sveltets_Render<TData>['props']> & {…}): …;
}
const BarChart__SvelteComponent_: $$IsomorphicComponent = null as any;
type BarChart__SvelteComponent_<TData> = InstanceType<typeof BarChart__SvelteComponent_<TData>>;
export default BarChart__SvelteComponent_;
```

Key pattern: the `class __sveltets_Render` is declared **at module
scope**, but its methods *return* values whose types come from
`$$render`'s return. `ReturnType<Render<T>['props']>` thus extracts
the body-scoped `$$Props` type *through* the render function's
scope. Body-local `typeof X` refs resolve naturally because TS
resolves them at the point `$$render` was declared — inside
`$$render`'s own scope, where `handler`/`labels`/etc are in scope.

## Fixture dirs

Each dir has a `tsconfig.json` + one or two `.ts` files (standalone,
compiled via tsgo). Run tsgo against each dir to verify the gate:

```sh
./node_modules/.bin/tsgo -p design/class_wrapper/<dir>/tsconfig.json
```

- **`current/`** — reproduces the TS7006 failure on the bare
  `$$render` + `declare const __svn_component_default: …` shape we
  emit today. Compilation should report the expected diagnostic;
  the point is to confirm the shape fails where we observe it
  failing.
- **`fixed/`** — the class-wrapper shape rewritten with
  `ReturnType<Render['props']>` extracting Props through the render
  function's scope. Compilation **must** report zero diagnostics.
  This is the target the Rust emit must produce.
- **`broken/`** — the fixed shape with a deliberate prop-type
  mismatch at the consumer site, proving the new shape still
  catches real errors at the right source position (not silently
  widening everything to `any`).

## After all three compile as expected

Only after the fixtures gate green (fixed: 0 errors; current:
TS7006; broken: exact expected diagnostic) does Phase 2.2 — the
Rust implementation — begin.
