# SlotHandler port plan

**Status:** Stages 1-7 landed 2026-04-29.

## Implementation summary

| Stage | Status | Commit |
| --- | --- | --- |
| 1. Slot-def model + writer | ✓ landed | `a3e747f6` |
| 2. Each/await resolver | ✓ landed | `7ac6605f` |
| 3. OXC slot-attr rewriter | ✓ landed | `6e733920` |
| 4. Let-forwarded slot resolution | ✓ landed | `7ac6605f` |
| 5. `$$Slots` interface override | ✓ landed | `63b5ed4f` |
| 6. Consumer-side audit | ✓ no changes needed | — |
| 7. Snapshot/bench review | ✓ 2 upstream-mirror files updated | — |

Native Stage 4 diverges from the PLAN's value-style recommendation:
emits `undefined as any as (__SvnComponentSlots<typeof C>['default']['foo'])`
(TYPE position, extracts S via `infer S` from `SvelteComponent<P, E, S>`)
instead of upstream's `__sveltets_2_instanceOf(C).$$slot_def['default'].foo`
(VALUE position). Native is stricter — upstream's projection collapses
to `any` via `SvelteComponent`'s `[prop: string]: any` index signature,
while native's mapped-type inference threads the typed slot map through.

Bug fixtures validating each stage:
- 116-each-slot-binding-resolved (Stage 2)
- 117-let-forwarded-slot (Stage 4)
- 118-slot-attr-member-expression (Stage 3)
- 119-strict-slots-interface (Stage 5)

Stage 6 audit confirmed:
- Default slot destructures use the current component instance
  (`emit_let_slot_destructure` called with the inst).
- Named-slot child destructures use the parent instance
  (`try_emit_slot_let_consumer_open` takes parent_inst).
- `<svelte:component>` / `<svelte:self>` paths hoist the inst when
  `child_is_slot_let_consumer` matches (round-4 #6 fix).
- `$$_$$` dummy uses the immediate void-read pattern, no
  `VoidRefRegistry` entry needed.

---

**Original plan:** refreshed 2026-04-29 after comparing the current
native code against the pinned upstream `language-tools` submodule.

**Goal:** port upstream `SlotHandler` behavior so producer slot definitions
are resolved through template scope instead of emitted from shadowed template
locals. This is the remaining large slot-parity gap. The concrete failure class
is:

```svelte
<TooltipContext let:tooltip>
  <slot {tooltip} />
</TooltipContext>
```

The generated `slots.default.tooltip` type must come from
`TooltipContext.$$slot_def.default.tooltip`, not from a module prop/import named
`tooltip` and not from an unresolved template local.

The old plan mixed current work with pre-existing code. This version treats
upstream as source of truth but uses the native architecture that already
exists.

---

## 1. Upstream behavior to match

Primary upstream files:

- `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/slot.ts`
- `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/TemplateScope.ts`
- `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/handleScopeAndResolveForSlot.ts`
- `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/index.ts`
- `language-tools/packages/svelte2tsx/src/svelte2tsx/createRenderFunction.ts`

Upstream has two cooperating pieces:

1. `TemplateScope` records active template bindings.
   - name membership
   - binding init node
   - binding owner node
   - parent scope lookup

2. `SlotHandler` records resolved slot attributes.
   - `slots: Map<slotName, Map<attrName, resolvedExpression>>`
   - `resolved: Map<bindingInit, resolvedExpression>`
   - expression rewriting from template-local identifiers to
     scope-independent expressions

Important upstream resolution rules:

- `{#each items as item}` resolves `item` through `items`.
- each destructures resolve with an IIFE:

  ```ts
  (({ a }) => a)(__sveltets_2_unwrapArr(items))
  ```

- each index resolves as a number-like value.
- `{:then value}` resolves through the awaited expression.
- `{:catch error}` resolves to `any`.
- `<Component let:foo>` resolves as:

  ```ts
  __sveltets_2_instanceOf(Component).$$slot_def['default'].foo
  ```

- `<Inner slot="name" let:foo>` resolves from the parent component instance's
  named slot:

  ```ts
  parent.$$slot_def['name'].foo
  ```

- `<slot>` attrs are resolved at the slot site:
  - text attrs stay string literals
  - expression attrs are AST-rewritten
  - shorthand attrs are AST-rewritten
  - object shorthand becomes `key: resolvedValue`
  - member properties and object keys are not rewritten
  - spreads are preserved as spreads

- `interface $$Slots` or `type $$Slots` in the instance script overrides the
  generated slot literal in the render return.

Upstream puts the final result into the render function return:

```ts
return {
  props: ...,
  events: ...,
  slots: { 'default': { tooltip: resolvedTooltip } }
}
```

That final `slots` object is emitted outside the original template scope, so
every template-local identifier must already have been rewritten.

---

## 2. Native code that already exists

Do not add a second copy of upstream `TemplateScope`. Native already has the
right starting points:

- `crates/analyze/src/template_scope.rs`
  - already walks scope-introducing template constructs
  - already parses binding patterns with OXC
  - already supports each, await, snippets, and let directives

- `crates/analyze/src/template_walker.rs`
  - already collects `TemplateSummary.slot_defs`
  - currently uses a simple `ShadowStack`
  - currently skips slot attrs when the leading identifier is shadowed
  - currently does not resolve shadowed identifiers

- `crates/emit/src/props_emit.rs`
  - already owns `write_slots_field_type`
  - currently emits slot attr expressions by slicing source or stringifying
    literals

- `crates/emit/src/render_function.rs`
  - TS overlay path already returns `slots: ...`
  - JS overlay path still returns only `props`; JS slot parity is separate

- `crates/emit/src/nodes/let_directive.rs`
  - consumer-side `<Component let:foo>` destructures already exist
  - named slot child destructures already mostly exist
  - do not reimplement this from scratch

- `crates/typecheck/src/svelte_shims_core.d.ts`
  - component instances already carry `SvelteComponent<P, E, S>`
  - avoid rewriting this around `__SvnInstance<P, S>` unless a fixture proves
    it is necessary

Current native gaps:

- slot producer attrs are not resolved through template scope
- the slot attr expression path still uses a leading-identifier scanner
- slot defs cannot represent spreads cleanly
- slot defs cannot distinguish value expressions from type expressions
- `$$Slots` does not override generated slots
- let-forwarded producer slots still need an upstream-equivalent
  `instanceOf`/slot-def access path

---

## 3. Correct target architecture

### 3.1 Keep one scope walker

Extend `crates/analyze/src/template_scope.rs`; do not create a new
`TemplateScope` module.

The existing walker is enough for shadowing, but not enough for full
SlotHandler parity because the callback currently loses owner/init context.
Add richer scope payloads so the analyzer can know why a binding exists.

Needed binding owners:

```rust
enum SlotBindingOwner {
    EachItem { items_range: Range },
    EachIndex,
    AwaitThen { promise_range: Range },
    AwaitCatch,
    Let {
        component_ref: ComponentSlotSource,
        slot_name: SmolStr,
        prop_name: SmolStr,
    },
    UnknownShadow,
}
```

`UnknownShadow` is important. If a binding is scoped but cannot be resolved
with upstream semantics, keep it shadowed instead of falling back to a
module-level identifier.

### 3.2 Replace `ShadowStack` with a resolver stack

In `template_walker.rs`, replace the current active-name-only `ShadowStack`
with a stack that maps:

```rust
name -> Option<ResolvedSlotExpr>
```

Meaning:

- `Some(expr)`: this template local has a safe scope-independent replacement.
- `None`: this name is shadowed but not safely resolvable; do not emit a
  module-scope fallback for it. If a slot attr expression depends on this
  name, treat the whole attr as unresolved and skip it unless a fixture proves
  an `any` fallback is safer.
- missing: normal module/import/prop identifier; leave it as source text.

This preserves upstream's critical fallback behavior without reintroducing the
bug where a template local accidentally resolves to a same-named module value.

### 3.3 Change the slot-def data model

The current shape is too small:

```rust
Vec<(SmolStr, SlotAttrExpr)>
```

It cannot represent spreads and it assumes every expression can be emitted as a
plain value expression.

Use an ordered shape:

```rust
pub struct SlotDef {
    pub slot_name: SmolStr,
    pub attrs: Vec<SlotAttr>,
}

pub enum SlotAttr {
    Prop {
        name: SmolStr,
        expr: ResolvedSlotExpr,
    },
    Spread {
        expr: ResolvedSlotExpr,
    },
}

pub enum ResolvedSlotExpr {
    Value(String),
    Type(String),
}
```

Writer rule:

- `Value("expr")` emits `(expr)`
- `Type("T")` emits `(undefined as any as (T))`

Do not use `BTreeMap` for the primary representation. Upstream `Map` behavior
is order-sensitive enough that sorted output can hide bugs. Use `Vec` plus an
explicit last-slot-wins replacement policy for duplicate slot names.

### 3.4 Prefer type expressions for each/await where practical

The slice-1 fixture under
`design/slot_handler/fixtures/01-each-binding-resolved/` validated that native
can avoid new unwrap shims for each bindings by emitting type assertions.

Use these native forms unless a fixture proves the upstream value form is
needed:

```ts
undefined as any as (
  typeof items extends Iterable<infer __svn_T> ? __svn_T : never
)
```

```ts
undefined as any as Awaited<typeof promise>
```

```ts
undefined as any as any
```

For let-forwarding, keep the upstream value-style shape because it naturally
projects from component slot defs:

```ts
__svn_instanceOf(Component).$$slot_def['default'].tooltip
```

### 3.5 AST-rewrite slot attr expressions

Add an OXC-based expression rewriter for slot attrs. This replaces the current
leading-identifier scanner.

Required behavior:

- parse the attr expression as an expression
- walk identifiers
- skip member properties
- skip object keys
- rewrite object shorthand from `{ value }` to `{ value: resolvedValue }`
- replace identifiers that have `Some(ResolvedSlotExpr)` in the resolver stack
- mark the expression unresolved if it touches an identifier that has `None`
  in the resolver stack
- leave missing identifiers unchanged
- apply source replacements from right to left

This belongs in analyze, not emit. Emit should only serialize the already
resolved slot-def data.

### 3.6 Keep consumer destructure work small

Consumer-side destructuring is already implemented in
`crates/emit/src/nodes/let_directive.rs`.

The remaining work is an audit, not a rewrite:

- verify default slot destructures use the current component instance
- verify named slot child destructures use the parent instance
- verify `<svelte:component>` and `<svelte:self>` still hoist instances when
  slot lets are present
- keep the `$$_$$` dummy local pattern if needed, but do not add a
  `VoidRefRegistry` entry unless the current immediate read proves insufficient

### 3.7 Add `$$Slots` override

Add AST-based detection for `interface $$Slots` and `type $$Slots` in the
instance script.

Rules:

- instance script declaration counts
- module script declaration does not count
- comments and strings do not count
- generic type aliases count

When present, the render return should use `$$Slots` instead of generated slot
defs, matching upstream's `uses$$SlotsInterface` behavior.

Native can emit either of these shapes if the fixture validates the same type
behavior:

```ts
slots: {} as unknown as $$Slots
```

or:

```ts
slots: undefined as any as $$Slots
```

Prefer the upstream-looking shape unless it conflicts with existing native
render-return conventions.

### 3.8 Add only the shim surface required by fixtures

Expected shim addition:

```ts
declare function __svn_instanceOf<T>(component: { new (...args: any[]): T }): T;
```

Add overloads only when a fixture proves they are needed for native iso
components or legacy `SvelteComponentTyped` components.

Avoid broad changes to `__svn_ensure_component` until a failing fixture proves
the current `SvelteComponent<P, E, S>` channel erases slots.

---

## 4. Fixture plan

The current `01-each-binding-resolved` fixture is a useful seed, but it is only
slice 1. Add focused fixtures before Rust behavior changes.

### 4.1 Producer each binding

Producer:

```svelte
{#each items as item, index}
  <slot {item} {index} />
{/each}
```

Expected slot types:

- `item` derives from `items`
- `index` is `number`
- typo in consumer fails at the source position

### 4.2 Producer await binding

Producer:

```svelte
{#await promise then value}
  <slot {value} />
{:catch error}
  <slot {error} />
{/await}
```

Expected:

- `value` derives from `Awaited<typeof promise>`
- `error` is `any`

### 4.3 Destructured each binding

Producer:

```svelte
{#each rows as { id, meta }}
  <slot {id} {meta} />
{/each}
```

Expected:

- IIFE-like destructure behavior or equivalent type projection
- `id` and `meta` resolve from the array element, not module scope

### 4.4 Let-forwarded slot prop

Producer:

```svelte
<Context let:tooltip>
  <slot {tooltip} />
</Context>
```

Expected:

```ts
slots: {
  default: {
    tooltip: __svn_instanceOf(Context).$$slot_def['default'].tooltip
  }
}
```

This is the target bug class.

### 4.5 Named slot child with let

Consumer shape:

```svelte
<Parent>
  <div slot="footer" let:page>
    {page.title}
  </div>
</Parent>
```

Expected:

- child body destructures from the parent instance's `$$slot_def['footer']`
- producer named slot emits `footer`

### 4.6 Slot attr spreads and object shorthand

Producer:

```svelte
{#each rows as row}
  <slot {...row} data={{ row }} />
{/each}
```

Expected:

- spread survives as a spread in the slot object
- shorthand/object identifiers are resolved through `row`

### 4.7 Shadow fallback

Producer:

```svelte
<script>
  export let tooltip;
</script>

<Context let:tooltip>
  <slot {tooltip} />
</Context>
```

Expected:

- slot `tooltip` resolves through `Context`
- it never falls back to the exported prop

### 4.8 `$$Slots`

Producer:

```svelte
<script lang="ts">
  interface $$Slots {
    default: { item: { id: string } };
  }
</script>

<slot item={123} />
```

Expected:

- render return slots are `$$Slots`
- generated slot attrs do not override the declared interface
- decide in this fixture whether native should also validate producer attrs
  against `$$Slots` now or defer that to a separate parity pass

---

## 5. Staged implementation

Each stage should be small enough to review by emitted shape against upstream.

### Stage 0 - Fixture refresh

Update `design/slot_handler/fixtures/` with the cases in section 4.

No Rust behavior changes in this stage.

### Stage 1 - Slot-def model and writer

Change analyze/emit data structs to support:

- ordered slot definitions
- prop attrs
- spread attrs
- value expressions
- type expressions

Keep current behavior by converting existing `Range`, `Shorthand`, and
`Literal` into the new model without new resolution.

Update `write_slots_field_type` in `crates/emit/src/props_emit.rs`.

### Stage 2 - Resolver stack for each/await

Extend `template_scope.rs` callbacks so each/await bindings carry enough
source context to build `ResolvedSlotExpr`.

Replace `ShadowStack` with the resolver stack in `template_walker.rs`.

Implement:

- each item
- each index
- await then
- await catch
- destructured each/await patterns

At the end of this stage, producer slots inside each/await should no longer
emit unresolved template locals.

### Stage 3 - OXC expression rewriter

Add the AST expression rewriter and switch `<slot>` attr collection to it.

This stage should remove the character-level leading identifier path for slot
attrs.

Required attr support:

- expression
- shorthand
- literal
- spread
- object shorthand inside expressions

### Stage 4 - Let-forwarded producer slots

Add scope resolution for `<Component let:foo>` and named-slot `let:` sources.

This is the most important parity stage.

Expected output shape:

```ts
__svn_instanceOf(Component).$$slot_def['default'].foo
```

or:

```ts
__svn_instanceOf(Component).$$slot_def['name'].foo
```

Add the minimal `__svn_instanceOf` shim needed by the fixtures.

### Stage 5 - `$$Slots`

Add AST detection for instance-script `interface $$Slots` and
`type $$Slots`.

Thread the flag into render-return slot emission and override generated slot
defs when present.

### Stage 6 - Consumer audit

Audit the existing code in `crates/emit/src/nodes/let_directive.rs` and related
component/element emit paths.

Do not rewrite it unless a fixture fails. The expected work is limited to
small fixes around parent instance selection and special components.

### Stage 7 - Snapshot and bench review

Accept snapshot churn only after checking representative outputs against
upstream `expectedv2.ts` files.

Focus review clusters:

- components with `<slot>`
- components with `<Component let:...>`
- named slots
- each/await slots
- `$$Slots`

Bench review should include layerchart and datagrid because they exercise the
slot-let surface.

---

## 6. Things not to do

- Do not add a second `TemplateScope` implementation.
- Do not sort slot attrs or slot names with `BTreeMap` as the primary model.
- Do not revive a character-level scanner for JS/TS expressions.
- Do not rewrite consumer `let:` destructuring from scratch.
- Do not extend `__SvnInstance<P, S>` unless a fixture proves the current
  `SvelteComponent<P, E, S>` path erases slot types.
- Do not treat snippet params as upstream SlotHandler bindings without a
  fixture. Upstream's slot resolver does not obviously map snippet params the
  same way as each/await/let.
- Do not count module-script `$$Slots`.

---

## 7. Open risks

### R1 - Let-owner context

The existing template scope walker knows that a `let:` binding exists, but the
full resolver needs to know which component and slot define it. This may require
enriching let-scope callbacks or adding a small pre-scan around component child
walks.

### R2 - `__svn_instanceOf` overloads

The first shim can be narrow. If external legacy `SvelteComponentTyped`, native
iso components, or `svelte:component` values fail, add overloads fixture by
fixture.

### R3 - Duplicate slot names

Upstream's `Map.set(slotName, attrs)` means a later slot definition replaces an
earlier one for the same slot name. Native currently can emit duplicate object
keys. Match upstream deliberately.

### R4 - JS overlay slots

The TS path can reach parity first. JS overlays currently return only `props`,
so JS slot propagation is a separate pass.

### R5 - `$$Slots` validation

Upstream uses `$$Slots` as the render-return slot type. Whether native also
validates producer slot attrs against `$$Slots` in this pass should be decided
by fixture, not guessed.

---

## 8. Implementation checklist

1. Re-read `CLAUDE.md`.
2. Diff one upstream overlay for the target bug class.
3. Add/refresh fixtures.
4. Land the slot-def model and writer with no behavior change.
5. Land each/await resolver support.
6. Land OXC slot attr rewriting.
7. Land let-forwarded slot resolution and minimal shim.
8. Land `$$Slots` override.
9. Audit existing consumer destructures.
10. Review snapshot churn against upstream shapes.

If a stage regresses unrelated fixtures, stop and diff the emitted native file
against upstream for one representative case before changing the model again.
