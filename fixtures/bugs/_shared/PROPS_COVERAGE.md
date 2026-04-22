# Props-churn coverage map

Phase 1 of the architectural refactor (see `notes/PLAN.md`) centralised
every Props decision behind `svn_analyze::PropsInfo`. Before that, the
same logic was scattered across `find_props_type_source`, `find_props`,
`synthesize_props_type_from_export_let`, and an emit-side
`root_type_name` helper. Five historical commits land repairs that
exercise the four `PropsSource` branches; this file names the
fixture(s) that lock each repair so a future regression in
`PropsInfo::build` is caught on the first test run.

If you're porting a new Props-shape, add a bullet here pointing to the
fixture(s) that exercise it.

## `f45dbdd` — exports-object as props base for Svelte-4 callable generics

**What broke:** `<BarChart labels={{ format: (value) => … }} />`
fired TS7006 on the arrow parameter. Our overlay flattened the
callable-default's props slot to `Partial<Record<string, any> & …>`
whenever the user's `interface $$Props` referenced body-local
`typeof X`, killing contextual-type flow.

**Fixture coverage:** `52-export-let-props/` exercises the
Svelte-4 `export let foo: T`-driven props synthesis (which is the
input to the callable-generics arm fixed here). Snapshot:
`crates/cli/tests/emit_snapshots/bugs/52-…` (if present).

**PropsSource branch:** `SynthesisedFromExports`.

## `ed4d7ee` — arrow-signature type for untyped function-valued export let

**What broke:** `export let onChange = (v) => …` (no annotation)
synthesised `onChange?: any` — losing the arrow's parameter types
entirely. Consumers passing `<Comp onChange={(v) => …} />` saw the
prop context as `any` and every param fired TS7006 implicit-any.

**Fixture coverage:** `52-export-let-props/Legacy.svelte` includes
an `export let onChange: (value: string) => void;` declaration and
asserts the consumer's arrow parameter types. The arrow-signature
synthesis path is exercised by `PropsInfo::build`'s tests:
`arrow_signature_from_init` → `append_props_from_var_decl` →
`PropsSource::SynthesisedFromExports`.

**PropsSource branch:** `SynthesisedFromExports`.

## `a5f0036` — revert `props_type_root` force-hoist

**What broke:** Earlier commit `29bb8bd` always hoisted the user's
Props type even when its body referenced body-scoped locals via
`keyof typeof X`. This exposed the declare-const stub's
`keyof typeof X → string | number` widening as a TS7053 index-
signature error on real components (cnblocks `ProgressiveBlur.svelte`).

**Fixture coverage:** `57-hoisted-type-referencing-body-name/`
exercises the type-visibility decision that replaced the force-hoist.
Snapshot asserts the emit shape for a Props type whose body references
a body-local identifier.

**Related analyze state:** `PropsInfo::type_root_name` is the input
to `script_split`'s hoisting decision. The invariant that `type_root_name`
is `None` for literal shapes (which never need hoisting) and `Some`
only for named references is covered by
`root_type_name_of_handles_common_shapes` in `analyze/src/props.rs`.

## `821be47` / `1ec39e1` — surface component exports

**What broke:** `export function foo()` / `export const bar` in a
Svelte component weren't being surfaced on the default export's
`Exports` type. Consumers calling `instance.foo()` fired TS2339 on a
previously-undetected member.

**Fixture coverage:** `50-preserve-export-type/` locks the emit
shape for `export function` / `export const` surfacing through the
default export.

**PropsInfo role:** the exports-surfacing logic is adjacent to
Props (both read `export let`/`export const`/`export function` at
the program top level), but exports are *not* a PropsInfo concern —
they live on a separate `ExportedLocalInfo` collection in
`script_split`. Kept here because the two logical groups were
scattered in the same code region before the PropsInfo split.

## How to extend

1. Pick one of the four `PropsSource` branches the new shape
   exercises: `RuneAnnotation`, `RuneGeneric`, `LegacyInterface`, or
   `SynthesisedFromExports`.
2. Minimal `input.svelte` reproducing the failure under strict
   `noUnusedLocals` / `noImplicitAny`.
3. `expected.json` with `{"clean": true}` when it should pass, or
   `{"errors": [{code, line, column}]}` for deliberate-broken
   verification fixtures.
4. If the shape is also worth locking at the emit layer, add an
   `expected.emit.ts` snapshot under
   `crates/cli/tests/emit_snapshots/bugs/<NN>-<slug>/`.
5. Add a bullet to this file.
