# Phase A — emit-shape design (validated 2026-04-17)

Spec for the Phase B rewrite of `crates/emit/src/lib.rs`. Every shape
decision below is tsgo-validated against `design/phase_a/fixture/`,
which produces **exactly** four diagnostics — each one a deliberately-
induced error in `Errors.svelte.ts` — and zero spurious diagnostics in
the clean consumer (`App.svelte.ts`), the edge-case consumer
(`Edges.svelte.ts`), or the third-party-class-component consumer
(`Classes.svelte.ts`).

Rerun:

```sh
cd design/phase_a/fixture
../../../node_modules/@typescript/native-preview-darwin-arm64/lib/tsgo \
    --pretty false -p tsconfig.json
```

Expected output is the four lines at the bottom of this doc.

---

## The shape

**Component-as-callable default, consumed via a constructor wrapper.**
Each overlay exports a value typed as Svelte 5's `Component<Props>`:

```ts
declare const __svn_component_default: import('svelte').Component<{
    checked: boolean;
    onchange: (event: { checked: boolean }) => void;
}>;
export default __svn_component_default;
```

A component instantiation in the parent template emits as:

```ts
// <Switch checked={isOn} onchange={({ checked }) => (isOn = checked)} />
{
    const __svn_C_1a4 = __svn_ensure_component(Switch);
    new __svn_C_1a4({
        target: __svn_any(),
        props: {
            checked: isOn,
            onchange: ({ checked }) => (isOn = checked),
        },
    });
}
```

### Why this shape

Three constraints pinned the choice:

1. **Contextual typing has to flow from Props into callback
   destructures and snippet arrow params.** The old satisfies-
   Partial-ComponentProps path collapsed Props through a conditional
   extractor chain to `any`, which is why `onchange={({ checked }) =>
   ...}` fired TS7031 implicit-any in a-sveltekit-app. A callable shape where
   the parent signature carries Props directly is the only shape that
   preserves this flow in tsgo's strict mode.

2. **User code routinely does `ComponentProps<typeof Foo>`.** Real
   svelte's ComponentProps has the constraint `T extends
   SvelteComponent | Component<any, any>`. A default export typed as
   a plain class (extending a narrower SvelteComponent<Props>) fails
   `SvelteComponent<any>`'s invariant-in-Props constraint — TS2344.
   Typing the default as `Component<Props>` (function form) satisfies
   the Component<any, any> alternative directly.

3. **Third-party libraries ship Svelte-4-style classes.**
   lucide-svelte, phosphor-svelte, bits-ui export class components
   extending SvelteComponent. These aren't callable; emitting
   `Foo(anchor, {...})` on them fires TS2348 "Value of type 'typeof
   Foo' is not callable. Did you mean to include 'new'?". A unified
   emission that works for both our callable defaults and third-party
   classes is required.

The `__svn_ensure_component` helper is the adapter that bridges
constraints 1 and 3: four overloads cover the component-shape space
and return a constructible whose `props` slot carries Props wrapped in
`Partial<>`. The intermediate local (`const __svn_CN = ...`) is what
makes generic components' `<T>` resolve at the `new` site — TS binds
the construct signature's generics against the concrete prop values
there rather than at the `__svn_ensure_component` site, where only
the component type is visible. Without the local, generic
components' item types resolve to `unknown`.

### Partial<Props>

The synthesized constructor's `props?` slot is `Partial<Props>` —
required props stay OPTIONAL at the call site. This matters because
real components routinely receive props via bind: directives
(`bind:value={x}`), `{...spread}`, or an implicit `children` snippet
from the component's body — none of which show up in the emitted
object literal. Partial keeps the excess-property check (typo'd prop
names still fire TS2353) and contextual-typing flow (callback
destructures, snippet params); it only relaxes "all required props
present."

### Generics

Generic components work through the constructor wrapper — the
intermediate local carries the class's generic parameter forward:

```ts
// <VirtualList items={rows}>
//     {#snippet children(item)}<span>{item.label}</span>{/snippet}
// </VirtualList>
{
    const __svn_C_xx = __svn_ensure_component(VirtualList);
    new __svn_C_xx({
        target: __svn_any(),
        props: {
            items: rows,   // T inferred = typeof rows[number]
            children: (item) => {  // item: T
                void item.label;
                return __svn_snippet_return();
            },
        },
    });
}
```

TS binds the class's generic at the `new` site, seeing the concrete
`rows` shape and propagating it into the snippet's parameter type.

### Snippet children

Snippets that are direct children of a `<Parent>` emit as
arrow-callback values on the constructor's `props`:

```ts
{
    const __svn_C_yy = __svn_ensure_component(Wrapper);
    new __svn_C_yy({
        target: __svn_any(),
        props: {
            items: rows,
            row: ({ id, label }) => {
                void id;
                void label;
                // ... snippet body
                return __svn_snippet_return();
            },
        },
    });
}
```

`__svn_snippet_return()` produces a value typed `any`, which satisfies
`Snippet`'s branded return shape without fighting it. No more
per-param `: any` injection (the D7 artefact).

### Element bind:this

The `__svn_bind_this_check<El>` helper is declared in the shim but
not currently emitted — the pre-refactor definite-assignment rewrite
(wrapping the user's `let el: T;` in `!`) remains in place, and no
test in the suite regressed on its absence. Wiring up the emit of
`__svn_bind_this_check<TagType>(el)` for every `bind:this={el}` site
is deferred until a bench repo demonstrates a concrete need.

### Stores / each / if / @const

Unchanged from pre-refactor emit. Compose normally inside the
`__svn_tpl_check` body.

---

## The helper set

| Name                          | Role                                                     |
| ----------------------------- | -------------------------------------------------------- |
| `__svn_any<T>()`              | Fresh `any` placeholder — target slot in `new` call.     |
| `__svn_ensure_component<C>()` | Normalize class or callable to a constructible. 4 overloads. |
| `__svn_each_items<T>(v)`      | Iterable wrapper for `{#each}`.                          |
| `__SvnEachItem<T>`            | Distributes item type for `__svn_each_items`.            |
| `__svn_bind_this_check<El>`   | (declared, not emitted yet) bind:this type assertion.    |
| `__svn_snippet_return()`      | Opaque branded-satisfying return for snippet arrow body. |
| `__SvnProps<C>`               | Extract props slot from either class or callable. Unwraps Partial<>. |
| `__SvnStore<T>`               | Structural store shape.                                  |
| `__SvnStoreValue<S>`          | Type-level store unwrap.                                 |
| `__svn_type_ref<T>()`         | Keeps a template-only `import type` alive.               |

Retired from the shim in B.2:
- `__SvnComponentProps<T>` — the broken satisfies-path extractor.

---

## Rejected alternatives

- **`Comp(anchor, props)` callable form as final emit.** First
  iteration. Works for our own typed callable defaults but breaks on
  third-party class components (TS2348 "not callable"). Also breaks
  if a user-declared context types something as `Component<Props>`
  (fixture 51-if-block-function-operand).

- **`new Comp({target, props})` without a wrapper.** Works for class
  components but breaks on our callable-typed defaults (TS7009 "new
  expression... implicitly has type 'any'"). No unified shape.

- **`class __svn_component_default extends SvelteComponent<Props>`
  default-export.** Consumer code writing `ComponentProps<typeof
  Foo>` fails `SvelteComponent<Record<string, any>, any, any>`
  invariant-in-Props constraint (TS2344). User-facing regression.

- **`Comp(anchor, {...})` callable plus `new Comp(...)` for classes
  using a conditional-type dispatcher helper (`__svn_call`) over
  `ConstructorParameters<C> | Parameters<C>`.** Breaks generic
  inference — TS can't resolve the class's `<T>` through the
  conditional before seeing the concrete call. VirtualList's snippet
  arrow reads `item: unknown`.

- **`__sveltets_2_*` naming (upstream's).** Explicitly forbidden by
  project rule 1 (no upstream naming).

---

## Expected tsgo output

```
src/Errors.svelte.ts(31,55): error TS2322: Type 'string' is not assignable to type 'boolean | undefined'.
src/Errors.svelte.ts(37,83): error TS2339: Property 'nope' does not exist on type '{ checked: boolean; }'.
src/Errors.svelte.ts(44,90): error TS2353: Object literal may only specify known properties, and 'foo' does not exist in type 'Partial<{ checked: boolean; onchange: (event: { checked: boolean; }) => void; }>'.
src/Errors.svelte.ts(55,40): error TS2339: Property 'missing' does not exist on type '{ id: number; label: string; }'.
```

Four errors, one per deliberate bug, each at the call site with the
right TS code. Clean files (App, Edges, Classes, Switch, Wrapper,
VirtualList, Lucide, store, svn_shims, svelte_shim) produce zero
diagnostics.

If Phase B or later emission ever drops below or rises above these
four, the shape has drifted.
