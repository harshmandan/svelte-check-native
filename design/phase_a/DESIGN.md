# Phase A — emit-shape design (validated 2026-04-17)

Spec for the Phase B rewrite of `crates/emit/src/lib.rs`.
Every shape decision below is tsgo-validated against
`design/phase_a/fixture/`, which produces **exactly** four diagnostics —
each one a deliberately-induced error in `Errors.svelte.ts` — and zero
spurious diagnostics in the clean consumer (`App.svelte.ts`) or the
edge-case consumer (`Edges.svelte.ts`).

Rerun at any time:

```sh
cd design/phase_a/fixture
../../../node_modules/@typescript/native-preview-darwin-arm64/lib/tsgo \
    --pretty false -p tsconfig.json
```

Expected output is the four lines listed at the bottom of this doc.

---

## The shape

**Component as callable.** Every `.svelte` file's overlay exports a
function-typed default whose second parameter carries the Props type:

```ts
declare function __svn_component_default(
    __anchor: any,
    props: {
        checked: boolean;
        onchange: (event: { checked: boolean }) => void;
    },
): any;
export default __svn_component_default;
```

A component instantiation in the parent template emits as a call:

```ts
// <Switch checked={isOn} onchange={({ checked }) => (isOn = checked)} />
Switch(__svn_any(), {
    checked: isOn,
    onchange: ({ checked }) => (isOn = checked),
});
```

This is the minimum shape that makes TypeScript's contextual typing flow
the parent's prop signature into the callback's parameter destructure.
The old `({...} satisfies Partial<__SvnComponentProps<typeof X>>)` shape
collapsed the prop type to `any` through the `Partial<>` +
conditional-extractor chain; see REFACTOR.md for the forensic.

### Generics

Generic components use a generic call signature:

```ts
declare function __svn_component_default<T>(
    __anchor: any,
    props: {
        items: T[];
        children: import('svelte').Snippet<[T]>;
    },
): any;
```

`VirtualList(anchor, { items: rows, children: (item) => {...} })` infers
`T` from `items`, which then flows into the snippet arrow's parameter.
No explicit annotation needed.

Explicit generic binding also works: `VirtualList<Item>(anchor, {...})`
— the rare case where a consumer over-constrains `<VirtualList<Item>>`.

### Snippet children

Snippets that are direct children of a `<Parent>` emit as arrow-callback
values on the parent's props object. The parent's declared
`Snippet<[T1, T2]>` prop type contextually types the arrow's parameter
tuple:

```ts
Wrapper(__svn_any(), {
    items: rows,
    row: ({ id, label }) => {
        void id;
        void label;
        // ... snippet body here
        return __svn_snippet_return();
    },
});
```

`__svn_snippet_return()` produces a value typed `any`, which satisfies
`Snippet`'s branded return shape without fighting it. No more
`annotate_snippet_params` / `: any` injection (the D7 artefact).

### Element bind:this

One helper call, no pair-of-assignments pattern:

```ts
// <input bind:this={inputEl} />
__svn_bind_this_check<HTMLInputElement>(inputEl);
```

`__svn_bind_this_check<El>(target: El | null | undefined): void` asserts
that `inputEl`'s declared type is a subtype of `El | null | undefined`.
Accepts any of the four shapes a user might pick for a bind target
(`T`, `T | null`, `T | undefined`, `T | null | undefined`) and rejects
a wrong element type. Fixes a-sveltekit-app bug #3 ("HTMLElement | null not
assignable to HTMLElement | undefined") by not imposing a particular
null/undefined story on the user.

### Component bind:prop (two-way bind)

Pair pattern: one assignment in the call-site props direction
(user → prop), one local of the prop slot's type assigned back
(prop → user):

```ts
// <Switch bind:checked={isOn} />
Switch(__svn_any(), { checked: isOn, onchange: () => {} });
let __svn_bind_checked_0!: __SvnProps<typeof Switch>['checked'];
isOn = __svn_bind_checked_0;
void __svn_bind_checked_0;
```

The `!` definite-assignment is required because the local is read but
never assigned; tsgo would otherwise fire TS2454.

`__SvnProps<F>` is a tiny type-level extractor:

```ts
type __SvnProps<F> = F extends (anchor: any, props: infer P) => any ? P : never;
```

Works for monomorphic components. Bind:prop on a generic component is
an edge case we defer to Phase B once we see it in the wild.

### Stores

Unchanged from today. `__SvnStoreValue<typeof store>` keeps working
because the shape doesn't depend on component-call emission at all.

```ts
let $count!: __SvnStoreValue<typeof count>;
// ... $count is typed as the store's inner T
```

### Each / if / @const / @html

Unchanged. `__svn_each_items(items)` iteration + index-as-number +
`if (condition) { ... }` block + inline `const` declarations all
compose normally with the callable-shape component emit — they just
happen inside the `__svn_tpl_check` render function body.

---

## The helper set

Final roster, nine helpers. All justified:

| Name                         | Role                                                     |
| ---------------------------- | -------------------------------------------------------- |
| `__svn_any<T>()`             | Fresh `any` placeholder — component-call anchor arg.     |
| `__svn_each_items<T>(v)`     | Iterable wrapper for `{#each}`. (unchanged)              |
| `__SvnEachItem<T>`           | Distributes item type for `__svn_each_items`.            |
| `__svn_bind_this_check<El>`  | Assert `bind:this` target accepts the element type.      |
| `__svn_snippet_return()`     | Opaque branded-satisfying return for snippet arrow body. |
| `__SvnProps<F>`              | Extract props slot of a component-as-callable.           |
| `__SvnStore<T>`              | Structural store shape. (unchanged)                      |
| `__SvnStoreValue<S>`         | Type-level store unwrap. (unchanged)                     |
| `__svn_type_ref<T>()`        | Keeps a template-only `import type` alive. (unchanged)   |

Retired from `svelte_shims_core.d.ts`:
- `__SvnComponentProps<T>` — the broken extractor.

No rune ambients change in Phase B — `$state`, `$derived`, `$effect`,
`$props`, `$bindable`, `$inspect`, `$host` all keep their current
shapes. The D1 fix to `$state(null)` / `$state(undefined)` overloads
stays.

---

## What the overlay emit looks like end-to-end

A library component (conceptual Svelte → overlay TS):

```ts
// Switch.svelte → Switch.svelte.ts

async function $$render_switch() {
    let { checked, onchange }: {
        checked: boolean;
        onchange: (event: { checked: boolean }) => void;
    } = $props();

    async function __svn_tpl_check() {
        // element-attribute checks, bindings, etc. go here
    }
    void __svn_tpl_check;
    void checked;
    void onchange;
}
$$render_switch;

declare function __svn_component_default(
    __anchor: any,
    props: {
        checked: boolean;
        onchange: (event: { checked: boolean }) => void;
    },
): any;
export default __svn_component_default;
```

A consumer:

```ts
// App.svelte → App.svelte.ts
import Switch from './Switch.svelte.ts';

async function $$render_app() {
    let isOn = $state(false);
    async function __svn_tpl_check() {
        Switch(__svn_any(), {
            checked: isOn,
            onchange: ({ checked }) => (isOn = checked),
        });
    }
    void __svn_tpl_check;
    void Switch;
    void isOn;
}
$$render_app;

declare const __svn_component_default: any;
export default __svn_component_default;
```

The consumer's default-export is untyped (`any`) — the emit only
bothers to synthesize a typed callable when it sees a `$props()`
annotation in the script. Untyped scripts stay `any`, which is the
correct "silently accept anything" semantic for Svelte 4-style or
`<script>`-less components.

---

## Rejected alternatives

- **`new $$_Comp({target, props: {...}})` (upstream shape).** Works
  equally well in principle, but requires a class-typed default
  export, an `$$_Comp = ensureComponent(Comp)` wrapper helper, and a
  `target: __svn_any()` prop we never need. Callable form is leaner
  and produces the same contextual-typing flow.
- **`({...} satisfies ComponentProps<typeof X>)` (today's shape).**
  Constraint collapse through the extractor chain. See REFACTOR.md.
- **Class-typed default export + instance access.** Would require
  tracking `$$prop_def` — unnecessary ceremony when we can read Props
  directly off the call signature.

---

## Expected tsgo output from the fixture

```
src/Errors.svelte.ts(30,13): error TS2322: Type 'string' is not assignable to type 'boolean'.
src/Errors.svelte.ts(37,26): error TS2339: Property 'nope' does not exist on type '{ checked: boolean; }'.
src/Errors.svelte.ts(45,13): error TS2353: Object literal may only specify known properties, and 'foo' does not exist in type '{ checked: boolean; onchange: (event: { checked: boolean; }) => void; }'.
src/Errors.svelte.ts(52,32): error TS2339: Property 'missing' does not exist on type '{ id: number; label: string; }'.
```

Four errors, one per deliberate bug, each at the call site with the
right TS code. Clean files (App, Edges, Switch, Wrapper, VirtualList,
store, svn_shims, svelte_shim) produce zero diagnostics.

If Phase B emission ever drops below or rises above these four, the
shape has drifted.
