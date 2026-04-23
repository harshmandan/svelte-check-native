# Item 2 fixture-first: get/set bind helper

CLAUDE.md architecture rule #8: every new emit shape is
tsgo-validated on a hand-written fixture before the Rust
implementation begins. This directory holds the fixtures
gating Item 2 of `notes/TODO.md`.

## The problem we're solving

**Correctness gap A1** from the 2026-04-23 deep-dive: our emit
at `crates/emit/src/lib.rs:3280-3288` (`PropShape::GetSetBinding`
arm) turns `<Child bind:value={get, set} />` into

```ts
{ value: (get)() }
```

— the setter is dropped. `<Child bind:value={get, bad_set} />`
with a mismatched setter signature produces zero diagnostics
today.

**Upstream's shape** (`svelte2tsx/src/htmlxtojsx_v2/nodes/Binding.ts:179`):

```ts
{ value: __sveltets_2_get_set_binding(get, set) }
```

Helper signature at `svelte2tsx/svelte-shims-v4.d.ts:269`:

```ts
declare function __sveltets_2_get_set_binding<T>(
    get: (() => T) | null | undefined,
    set: (t: T) => void,
): T;
```

The helper's `T` is inferred once per call site; both the
getter's return type AND the setter's parameter type are
type-checked against it, and the return flows out to the
consumer-side prop slot. All three surfaces (getter / setter /
consumer) are cross-checked.

Our port uses an `__svn_get_set_binding` name per CLAUDE.md
architecture rule #6 (synthesized names prefixed `__svn_*`).
Semantics identical.

## Fixtures

All three reduce the pattern to a single `value: T` prop literal,
matching how `emit_component_call` will emit it.

| Dir | Shape | Expected tsgo | Gate |
|---|---|---|---|
| `current/` | Today's emit: `{ value: (get)() }` with a mismatched setter. | 0 diagnostics (proves the gap). | `EXIT=0`, zero output. |
| `fixed/` | Target emit: `{ value: __svn_get_set_binding(get, set) }` with matching signatures. | 0 diagnostics (proves the clean case). | `EXIT=0`, zero output. |
| `broken/` | Target emit + deliberately-wrong setter (`bad_set` takes `number`, value is `string`). | Exactly 1 × TS2322 on `Consumer.ts:24:9`. | `EXIT=2`, one line. |

Run each fixture with:

```sh
cd design/get_set_binding/<dir>
../../../node_modules/.bin/tsgo --noEmit -p .
```

Verified 2026-04-23 against
`@typescript/native-preview-darwin-arm64`.

## What lands in Rust after this

1. Add `__svn_get_set_binding` shim declaration to
   `crates/typecheck/src/svelte_shims_core.d.ts`.
2. Rewrite `emit/src/lib.rs::write_prop_shape`'s
   `PropShape::GetSetBinding` arm to emit
   `name: __svn_get_set_binding(<getter>, <setter>)`.
3. Extend `PropShape::GetSetBinding` in
   `crates/analyze/src/template_walker.rs:213` to carry
   `setter_range` (parser already provides it; this is a
   pass-through).
4. Add a DOM-side branch for `bind:X={getter, setter}` in
   `emit/src/lib.rs::emit_element_bind_checks_inline` that
   emits the same helper call.
