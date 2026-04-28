# Port plan — upstream `SlotHandler` + `TemplateScope` resolver

**Status:** research complete, slice-1 design fixture validated
(`design/slot_handler/fixtures/01-each-binding-resolved/`),
implementation not started.
**Target gap:** Sankey/+page.svelte +4 over-fire on layerchart
(notes/OPEN.md §1). Revisit when this is the last remaining gap;
otherwise defer.

**Refinement (2026-04-28):** the slice-1 design fixture under
`fixtures/01-each-binding-resolved/` validates a simpler resolution
shape than the upstream-mirror sketch in §2.4 below: instead of
emitting a `__svn_unwrap_arr(items)` helper, project the each-binding
directly as a TS conditional type — `(typeof items extends
Iterable<infer __svn_T> ? __svn_T : never)`. Same observable
semantics as upstream, no shim addition required. Tsgo-clean on
both the typed-clean and TS2339-on-typo break cases. The
implementation slices in §4 (Stage 4 onwards) should adopt the
conditional-type form for slot-def emission. The full port still
needs the consumer-side destructure (Stage 5) and the
`__svn_instanceOf` machinery for `<Component let:X>` (Stage 3 / §2.4)
— those slices are unchanged.

**Scope:** port the machinery that turns slot-attr expressions like
`<slot {tooltip}>` inside `<TooltipContext let:tooltip>` into
scope-independent strings like
`__sveltets_2_instanceOf(TooltipContext).$$slot_def['default'].tooltip`
— the thing that makes the type of `tooltip` inside Chart's own
`$$render` return carry from upstream, not from the shadowed
module-scope `export let tooltip`.

Every upstream citation below refers to the pinned submodule
(`language-tools/`). Every line pointing at our tree is
`crates/*/src/*.rs`.

---

## 1. Upstream algorithm summary

### 1.1 `TemplateScope` (language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/TemplateScope.ts:7-48)

A linked scope: `parent?: TemplateScope`, plus three maps keyed by
binding name:

- `names: Set<string>` — cheap membership check (inherited from
  parent on construction, line 15).
- `inits: Map<string, WithName>` — name → the AST node that
  introduced the binding (`let:foo` directive, `{#each items as x}`
  identifier, `{:then y}` identifier, etc.).
- `owners: Map<string, Node>` — name → the containing block/element
  (EachBlock, ThenBlock, CatchBlock, InlineComponent, Element).
  Used to discriminate which resolution strategy applies.

Lookup walks the chain: `getInit(name)` / `getOwner(name)` fall back
to `parent?.getInit(name)` / `parent?.getOwner(name)` (lines 40-47).
A `.child()` call (line 31) returns a fresh scope whose `parent` is
`this` — called on `enter` of each scope-introducing node.

### 1.2 `SlotHandler.resolve()` — identifier → scope-independent string

(language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/slot.ts:64-109)

Given an identifier def + its init expression + the scope at the
binding site, `getResolveExpressionStr()` maps via owner type:

- `CatchBlock` → literal string `"__sveltets_2_any({})"` (line 94).
- `ThenBlock` → `"__sveltets_2_unwrapPromiseLike(" + resolveExpression(initExpr, scope.parent) + ")"`
  (lines 99-102). `scope.parent` matters: the init expression lives
  in the *outer* scope, so it's rewritten there.
- `EachBlock` → `"__sveltets_2_unwrapArr(" + resolveExpression(initExpr, scope.parent) + ")"`
  (lines 103-106).
- Any other owner → returns `null`, caller falls through.

`resolveLet()` / `getResolveExpressionStrForLet()` (lines 129-161) is
the distinct `let:X` path — maps directly to
`${getSingleSlotDef(componentNode, slotName)}.${letNode.name}`,
which expands to
`__sveltets_2_instanceOf(Component).$$slot_def['slotName'].letName`
(lines 308-317).

Destructuring patterns (`let:x={{a, b}}` or `{#each items as {a, b}}`)
are wrapped in an IIFE shim:
`((${destructuring}) => ${identifier.name})(${resolved})` — the
callee resolves to the right type, the arrow applies the user's
destructure pattern. (lines 111-127, 129-144)

### 1.3 `handleSlot()` — per-slot-site resolution

(slot.ts:252-279) Called from the template walker each time a `<slot>`
element is entered. Walks the slot's attributes:

- Finds `name="..."` attr; default is `"default"` (line 254).
- For each non-`name` attr, three cases:
  - `Spread` → `scope.getInit(attr.expression.name)` → `resolved.get(init)`,
    pushed as `__spread__NAME` → resolved key.
  - `Text` literal → passed through as `"literal"` (line 274).
  - `MustacheTag` or `AttributeShorthand` → calls `resolveAttr()` →
    `resolveExpression()`.

`resolveExpression()` (lines 195-250) is the core mechanic. It walks
the expression AST looking at `Identifier` nodes (skipping member
props, object keys). For each identifier, `scope.getInit(name)` →
`this.resolved.get(init)` — if both succeed, overwrite the identifier
in a MagicString. If either returns undefined, the identifier passes
through unchanged (so `<slot {moduleScopeValue}>` where
`moduleScopeValue` is an import, not a let-binding, emits bare —
**fallback is critical**).

Result goes into `slots: Map<slotName, Map<attrName, resolvedExpr>>`.

### 1.4 `{#each}` / `{#await}` scope extension

(language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/handleScopeAndResolveForSlot.ts:10-86
+ their calls from `htmlxtojsx_v2/nodes/EachBlock.ts`,
`AwaitPendingCatchBlock.ts`, `InlineComponent.ts`)

On walker `enter` of each scope-introducing node:

1. `scope.child()` (caller stores the child scope into a context
   passed to children).
2. For each binding identifier in the node's pattern, call
   `handleScopeAndResolveForSlot({identifierDef, initExpression, owner, slotHandler, scope})`:
   - `scope.add(identifierDef, owner)` populates the maps.
   - `slotHandler.resolve(identifierDef, initExpression, scope)` —
     populates `resolved` with the correct `__sveltets_2_unwrapArr` /
     `__sveltets_2_unwrapPromiseLike` / `__sveltets_2_any` expression.
3. For destructured patterns (`{#each items as {a, b}}`),
   `resolveDestructuringAssignment()` stores the IIFE-wrapped form.

The `Let` path (handleScopeAndResolveForSlot.ts:42-86) does the
equivalent for `<Component let:X>`: walks letNode's expression (a
pattern), adds each name to scope with `owner=component`, records
`slotHandler.resolveLet()` for each. Both bare (`let:foo`) and
renamed (`let:foo={bar}`) and destructured (`let:foo={{a, b}}`)
forms are handled.

### 1.5 Integration into `$$render`'s return

(createRenderFunction.ts:125-139)

```ts
const slotsAsDef = '{' +
    Array.from(slots.entries())
        .map(([name, attrs]) => `'${name}': {${slotAttributesToString(attrs)}}`)
        .join(', ') +
    '}';
// ... writes `, slots: ${slotsAsDef}` into the return literal.
```

Because every identifier inside the attrs map was already rewritten
to a scope-independent expression during the walk, the literal goes
into the render function body *at module scope* and references
nothing from the template scope. That's the whole trick: the walker
rewrites at identifier emission time, the final literal is
position-independent.

### 1.6 Consumer-side destructure

(htmlxtojsx_v2/nodes/InlineComponent.ts:184-207,
htmlxtojsx_v2/nodes/Element.ts:184-207)

For a `<Component let:foo>` consumer, upstream emits

```ts
{const {/*Ωignore_startΩ*/$$_$$/*Ωignore_endΩ*/, foo} = $$_inst.$$slot_def.default; $$_$$;
  /* child fragment */
}
```

and for named-slot consumers (`<template slot="x" let:foo>`),

```ts
{const {/*Ωignore_startΩ*/$$_$$/*Ωignore_endΩ*/, foo} = $$_parentInst.$$slot_def["x"]; $$_$$;
  /* child fragment */
}
```

The `$$_$$` dummy is there purely so TS doesn't report "unused
destructure" on the whole binding list when all let-bindings are
unused — its `ignore_start/end` wrapper keeps source-map diagnostics
from surfacing it. We should mirror the pattern: register `$$_$$` in
`VoidRefRegistry` the same way we register every other synthesized
name.

### 1.7 Type channel through `SvelteComponent<P, E, S>`

(svelte-shims-v4.d.ts:42,208,267)

Upstream's component type carries `$$slot_def: S` as a phantom
field. For generic components, `S = ReturnType<__sveltets_Render<T>['slots']>`
(see Chart.svelte's tail: `__sveltets_Render<TData>['slots']()`).
For non-generic, `$$slot_def: any` on `ATypedSvelteComponent` (line
208) carries no signal; the typed literal in `$$render`'s return is
the only source of truth.

**Concretely: Chart.svelte's upstream emit (verified via diff-emit):**

```ts
slots: {'default': {
  tooltip: __sveltets_2_instanceOf(TooltipContext).$$slot_def['default'].tooltip,
  brush: __sveltets_2_instanceOf(BrushContext).$$slot_def['default'].brush,
  aspectRatio: __sveltets_2_instanceOf(LayerCake).$$slot_def['default'].aspectRatio,
  /* ... */
}}
```

Every identifier — even those with no visible each/await/let
context — is rewritten through the scope chain. The reason: they
were let-bindings from enclosing `<XContext let:foo>` wrappers, so
`scope.getInit('tooltip')` returns the let directive from
`<TooltipContext let:tooltip>`, whose `resolved` entry is
`__sveltets_2_instanceOf(TooltipContext).$$slot_def['default'].tooltip`.

---

## 2. Port sketch — which crates get which pieces

### 2.1 Parser — **no changes needed**

`svn-parser` already exposes every AST shape required
(`crates/parser/src/ast.rs`):

- `EachBlock` lines 443-464: `expression_range`, `as_clause` with
  `context_range` (binding pattern) + `index_range` + `key_range`.
- `AwaitBlock` lines 466-491: `ThenBranch.context_range`,
  `CatchBranch.context_range` — both `Option<Range>`.
- `Directive` with `DirectiveKind::Let` (line 278) and
  `DirectiveValue::Expression { expression_range }` (line 289).
  Tested by `attributes.rs:866-873`.
- `Node::Element { name: "slot" }` — no dedicated variant, walker
  just matches on name (template.rs:610-698).
- `Attribute::Shorthand` / `Attribute::Expression` / `Attribute::Spread`
  — all three shapes on `<slot>` attrs are already first-class.

**Only consideration:** destructure patterns (e.g.
`{#each items as {a, b}}`) and let-aliases (`let:foo={bar}`) come
through as `Range` — we'd have to parse those ranges via `oxc_parser`
during analyze (matching CLAUDE.md's rule #1: "No character-level
scanners; walk the AST"). `crates/analyze/src/props.rs` already
does oxc-based destructuring extraction for `$props()` — reuse that
machinery.

### 2.2 Analyze — new `SlotResolver` concern

**New module: `crates/analyze/src/template_scope.rs`**

A port of the two upstream classes:

```rust
pub struct TemplateScope {
    parent: Option<Rc<TemplateScope>>,
    names: AHashSet<SmolStr>,
    inits: AHashMap<SmolStr, BindingInit>,
    owners: AHashMap<SmolStr, Owner>,
}

pub enum BindingInit {
    EachItem { items_range: Range, /* of enclosing #each */ },
    EachIndex,
    ThenValue { promise_range: Range },
    CatchError,
    LetBinding { let_node: LetRef, component: ComponentRef, slot_name: SmolStr },
}

pub enum Owner { EachBlock, ThenBlock, CatchBlock, Component, Element, SvelteElement }
```

**New module: `crates/analyze/src/slot_resolver.rs`**

```rust
pub struct SlotResolver {
    // slotName → attrName → resolved expression source (string)
    slots: BTreeMap<SmolStr, BTreeMap<SmolStr, String>>,
    // identifier node id → resolved expression
    resolved: HashMap<BindingKey, String>,
}
```

Mirrors `SlotHandler` — `resolve()`, `resolveExpression()`,
`resolveDestructuringAssignment()`, `handleSlot()`. The one hot path
is `resolveExpression`: walks the slot-attr expression via oxc's
`Visit` trait (borrow the pattern from `crates/analyze/src/rune.rs`
or `crates/analyze/src/props.rs`), collects `Identifier` nodes, and
rewrites them from the scope via a `Vec<(span, replacement)>` list
that's applied post-walk to produce the resolved string.

**Template walker integration:**
`crates/analyze/src/template_walker.rs` gains `TemplateScope`
threading. Current walker is stateless over children; it grows a
`Rc<TemplateScope>` parameter that's forked on:

- `Node::EachBlock` enter — child scope; add `as_clause.context_range`
  identifier(s) with `Owner::EachBlock`, init =
  `EachItem { items_range: expression_range }`. Emit destructures
  via oxc pattern parsing.
- `Node::AwaitBlock` enter — child scopes per branch; `ThenBranch`
  → `Owner::ThenBlock`, init = `ThenValue`; `CatchBranch` →
  `Owner::CatchBlock`.
- `Node::SnippetBlock` enter — child scope; parameters become
  `Owner::Component`-ish (upstream maps snippet to this path via
  `handleScopeAndResolveForSlot.ts` indirectly — verify before
  shipping).
- `Node::Component` and `Node::Element` enter — if any
  `let:X` directives on the attributes, child scope with
  per-directive `Owner::Component` / `Owner::Element` and
  `BindingInit::LetBinding`.
- `Node::Element { name: "slot" }` enter — do NOT create a child
  scope. Instead call `SlotResolver::handle_slot(&attributes, &scope)`,
  which populates the `slots` map.

**Output on `TemplateSummary`:**

```rust
pub struct TemplateSummary {
    // ... existing fields ...
    pub slot_defs: SlotDefs, // new
}

pub struct SlotDefs {
    // slotName → [ (attrName, resolved expression string) ]
    // BTreeMap order is stable → snapshots deterministic
    pub entries: BTreeMap<SmolStr, Vec<(SmolStr, String)>>,
}
```

Plus a per-let-binding target registry so `emit` can generate the
consumer-side destructures:

```rust
pub struct LetBindingSite {
    pub component_start: u32,      // locate enclosing instantiation
    pub slot_name: SmolStr,        // "default" or the template's slot=""
    pub destructures: Vec<LetDestructure>, // one per let:X
}

pub enum LetDestructure {
    /// `let:foo` → consumer-side `{ foo } = inst.$$slot_def.default`
    Simple { name: SmolStr },
    /// `let:foo={bar}` → `{ foo: bar }`
    Renamed { orig: SmolStr, alias: SmolStr },
    /// `let:foo={{a, b}}` → `{ foo: { a, b } }` — emit source slice verbatim
    Destructured { orig: SmolStr, pattern_range: Range },
}
```

### 2.3 Emit — producer return + consumer destructure

**Emit change 1: `$$render` return's `slots:` field**

`crates/emit/src/lib.rs:1598` + `:1611` currently hardcode
`slots: undefined as any as {}`. Replace with a typed literal
constructed from `summary.slot_defs`:

```text
slots: undefined as any as { 'default': { nodes: __svn_unwrap_arr(...), links: ... }, ... }
```

Pre-existing pattern exists (`:1592` branch for the
`prop_type_source + generics` class-wrapper case) so this slots in
without changing the surrounding signature. If `slot_defs.entries`
is empty, keep emitting `{}`.

**Emit change 2: consumer-side `<Component let:X>` destructure**

`:4952-4964` currently emits `let name: any; void name;`. Rewrite
to upstream's pattern:

```ts
{const {/*Ωignore_startΩ*/$$_$$/*Ωignore_endΩ*/, foo, bar: alias} = $$_inst.$$slot_def.default; $$_$$;
  /* children */
}
```

The component's instance local (`$$_inst` → our `__svn_inst_N`)
must exist before the destructure. Today we only emit
`const __svn_inst_N = new __svn_CN(...)` when the component has
event directives (`emit_component_call` behavior — verify at
`:5024-5088`). Extend to also emit the instance local whenever
`LetBindingSite` says the consumer has let-directives.

Named-slot consumers (`<template slot="x" let:foo>` as a child of
`<Component>`) read from `$$_parentInst.$$slot_def["x"]` — we need
to propagate the parent's instance local into the template-walker
context (currently nothing flows down; add a
`parent_inst_local: Option<SmolStr>` field to the emit walker).

### 2.4 Shim changes

`design/phase_a/fixture/src/svn_shims.d.ts` (the reference shim we
validate fixtures against) — but the real emitted shim lives
inlined in `crates/emit/src/lib.rs`:

1. `__SvnInstance<P>` → `__SvnInstance<P, S = any>`, extend with
   `$$slot_def: S;`.
2. Extend the typed overload of `__svn_ensure_component` so the
   return type is `__SvnInstance<P, Slots>` where `Slots` comes
   from `ReturnType<...['slots']>`. For non-generic overlays,
   `$$slot_def: any` lets the plain destructure parse; for
   generic overlays the typed channel flows through the render
   class (see `use_class_wrapper` gate at `lib.rs:~5090`).
3. New helpers with `__svn_` prefix (Rule #6):

   ```ts
   declare function __svn_instanceOf<T>(
       type: { new (...args: any[]): T } | (new (...args: any[]) => T) | T
   ): T;
   declare function __svn_unwrap_arr<T>(arr: ArrayLike<T>): T;
   declare function __svn_unwrap_promise_like<T>(p: PromiseLike<T> | T): T;
   ```

   Mirrors upstream's `__sveltets_2_instanceOf` / `_unwrapArr` /
   `_unwrapPromiseLike` (shim lines 63, 131-132).

   `__svn_instanceOf` has to accept BOTH component constructors
   (`typeof Component`) AND isomorphic components
   (`import('svelte').Component<P>`). Overloads needed — upstream
   does this via `AConstructorTypeOf<T>` plus the implicit
   isomorphic pathway in `__sveltets_2_ensureComponent`. Port the
   overload set as hand-written TS and gate-check with a fixture
   before writing Rust emit (Rule #8).

4. Register `$$_$$` in `VoidRefRegistry` the same way
   `__svn_tpl_check` is registered (template_walker.rs:314). One
   shared name per component; emit keeps a single `void $$_$$;`
   trailer in the consolidated block.

### 2.5 Fixture-first validation (Rule #8)

Before any Rust change, `design/slot_handler/fixtures/` gets:

- **`01-basic-let/`** — `Parent.svelte` with `<slot {foo}>` at
  producer; `Consumer.svelte` with `<Parent let:foo>` using `foo`.
  Hand-write the expected `.ts` overlays showing `slots:
  {'default': {foo: ...}}` on producer + destructure on consumer.
  Tsgo clean on the pair. Deliberate break: consumer writes
  `foo.nonexistent.x` → exact TS2339 at known position.

- **`02-each-in-slot/`** — `<slot {x}>` inside `{#each items as
  item}` where `x` = `item.value`. Expected producer emit has
  `__svn_unwrap_arr(items).value`. Break: consumer writes wrong
  type → TS2322.

- **`03-await-then-catch/`** — `<slot {v}>` inside `{#await p
  then v}` AND `<slot {e}>` inside `{#await p catch e}`. Expected:
  `__svn_unwrap_promise_like(p)` and `__svn_any({})`. Break cases
  for each.

- **`04-shadowed-let/`** — THE bug. A module-scope
  `export let tooltip` AND a template `<XContext let:tooltip>
  <slot {tooltip}>`. Verify the resolved expression is
  `__svn_instanceOf(XContext).$$slot_def['default'].tooltip`, not
  the module-scope export. Validator: tsgo clean on typed slot
  consumer, TS2339 if consumer writes `.wrongmember`.

- **`05-named-slots/`** — `<slot name="footer" {page}>` forwarded
  through `<template slot="footer" let:page>`. Verify both
  producer (`slots: {'footer': {page: ...}}`) and consumer
  destructure from `.parentInst.$$slot_def["footer"]`.

- **`06-nested-scopes/`** — `{#each outer as row}{#each row as
  cell}<slot {cell}>{/each}{/each}`. Expected:
  `__svn_unwrap_arr(__svn_unwrap_arr(outer))`.

- **`07-destructured-let/`** — `<Component let:{a, b}>` destructure
  pattern. Verify IIFE-wrapped resolution shape
  `(({a, b}) => a)(__svn_instanceOf(Component).$$slot_def['default'].<orig>)`.

All seven must gate green BEFORE any analyze/emit Rust change.
Invariant from Rule #8: if the hand-written TS doesn't produce
zero-error clean + exact-error break, the theory is wrong.

---

## 3. Blast radius

### 3.1 Snapshot corpora (`cargo test --test emit_snapshots`)

- **Producer-side (`$$render` return)**: every component with `<slot>`
  gets a new `slots:` literal. Estimated ~30 snapshots in
  `crates/cli/tests/emit_snapshots/htmlx2jsx/` (samples named
  `component-*-slot*`, `if-nested-slot-let`, `svelte-fragment`,
  etc. — 35 snapshots currently contain `__svn_each_items` or
  `let ` markers). All mechanical.

- **Consumer-side destructures**: 17 snapshots currently have
  `let X: any;` placeholders (grep `let [a-z_]+: any` across
  emit_snapshots/). All rewrite to `const { $$_$$, X } = inst.$$slot_def.default;`
  form. Mechanical.

- **svelte2tsx corpus (165 snapshots)**: most don't touch slots,
  but anything with `<Component let:>`, `<slot>`, `{#each}` in a
  slot position, or awaits shifts. Target: ≤50 snapshots changed
  (caught as review signal).

Every snapshot change is auto-accepted via
`UPDATE_SNAPSHOTS=1 cargo test --test emit_snapshots`; the PR is
reviewed shape-first (does the new form match upstream's pattern?).

### 3.2 Bench fleet (non-test; interactive discovery)

`let:` directive usage across bench/:

| Bench | Files with `let:` | Risk |
| :-- | --: | :-- |
| layerchart/packages/layerchart | (many — Chart, Tooltip, Brush, Geo) | TARGET. Expect 28 → 26 on success; risk = regression if scope resolver misses a case |
| datagrid/sites/official | 43 | HIGH. 43 files to re-type-check; any missed resolution path surfaces |
| palacms | 2 | LOW |
| cnblocks / cryptgeon / inference-playground / cobalt / pixzip-lite | 0 each | Unaffected |

Datagrid is the real risk surface. Run diff-emit on 3-5 datagrid
files during implementation (before locking behavior) and verify
the resolved expressions match upstream's shapes on each.

### 3.3 Integration suites

- `bug_fixtures/` + `v5_fixtures/` — should not regress (we don't
  delete fixtures, only emit changes). Any new fixture added for
  slot-let specifically lives under `bug_fixtures/<NN>-slot-let-*`.
- `upstream_sanity` (via test-sanity.js) — upstream's own tests
  cover slot-let comprehensively. The passing subset may grow;
  currently-failing SvelteKit-ambient cases stay scoped out.

---

## 4. Staged rollout

Each step ends in a green `cargo test --test emit_snapshots` and a
bench check that doesn't regress. Commit after each.

**Stage 0 — fixture gate.**
Write the seven `design/slot_handler/fixtures/` fixtures. All seven
must tsgo-clean + exact-break before anything else. No Rust yet.

**Stage 1 — analyze: `TemplateScope`.**
Add `crates/analyze/src/template_scope.rs`. Unit-tested via new
`#[cfg(test)]` module covering: each-block binding, each-block
destructure, await then/catch branches, let-directive shorthand +
alias + destructure. Zero behavior change to emit — purely
analyze-internal. Commit.

**Stage 2 — analyze: walk integration + `SlotDefs` population.**
Thread scope through `walk_template`. Populate `TemplateSummary.slot_defs`.
Assert the resolved strings match upstream's shape on a spot-check
of Sankey, Chart, TooltipContext. Still no emit change. Commit.

**Stage 3 — shim + new helpers.**
Extend `__SvnInstance<P, S=any>` in the emitted shim. Declare
`__svn_instanceOf`, `__svn_unwrap_arr`, `__svn_unwrap_promise_like`.
Register `$$_$$` in `VoidRefRegistry`. Verify the shim compiles
standalone via the tsgo fixture harness. Commit.

**Stage 4 — emit: producer-side `slots:` literal.**
Rewrite the `$$render` return (lib.rs:1598, :1611) to use
`summary.slot_defs` when non-empty. Emit snapshots move ~30 files —
review each for shape parity with upstream, accept via UPDATE_SNAPSHOTS.
Run layerchart bench: expect 28 ± small delta (consumer-side still
emits `let X: any`, so effect is partial). Commit.

**Stage 5 — emit: consumer-side destructure + instance local.**
Rewrite emit_component_call (lib.rs:4952, 5024-5088) to always emit
the instance local when LetBindingSite exists, then emit the
`$$_$$`-marker destructure. Walk parent-instance context into child
template walkers for named-slot consumers. Review snapshot churn.
Run layerchart bench: expect 28 → 26 (target achieved). Run
datagrid/palacms: expect ≤+3 variation. Commit.

**Stage 6 — cleanup + regression hunt.**
Delete the `let X: any;` placeholder path (old collect_let_directive_names
consumer, lib.rs:2558-2567). Re-run full bench fleet; mark any new
divergence as a new entry in `notes/OPEN.md`. Commit.

Between each stage, if any bench regresses by >2 errors, STOP, diff
the affected overlay vs upstream on one file, and read the scope
resolver output in the analyze phase. Don't guess.

---

## 5. Risks and kill criteria

### 5.1 Known risks

- **R1: Type inference for `__svn_instanceOf`.** Upstream's
  `__sveltets_2_instanceOf(Component)` works only when Component is
  a typed isomorphic component (the output of the `Component<P>`
  interface at module scope). If our shim overload set misses a
  case (bare `import SvelteComponent`, generic with constrained T,
  default-export alias), the resolved expression types as `any`
  and consumer destructures stay untyped — no type-safety win.
  Mitigation: fixture 04-shadowed-let covers the shape; fixture
  must fail-then-pass before Stage 3 ships.

- **R2: Named slot consumers propagate parent-instance context.**
  Our emit currently has no "parent component instance" state
  flowing to child template walk. Adding it threads through every
  `emit_template_node` callsite — ~8 callers. Mechanical but
  invasive.

- **R3: `<slot>` fallback content.** Svelte-5 snippets replace
  slots but Svelte-4 `<slot><Fallback/></slot>` has fallback
  children that still need to type-check. Stage 4 should verify the
  producer emit doesn't drop those — `handleSlot` upstream doesn't
  touch children, they walk separately.

- **R4: `$$Slots` interface declaration.** Users can declare
  `interface $$Slots { foo: { bar: string } }` in the script to
  hand-author slot types (upstream createRenderFunction.ts:125-126
  gates on `uses$$SlotsInterface`). When present, the synthesized
  slots literal is ignored and `{} as unknown as $$Slots` is emitted
  instead. Mirror this gate — one extra analyze-level flag.

- **R5: Snapshot volume.** If Stage 4 moves >50 snapshots in
  svelte2tsx, the review burden is high. Accept mechanically only
  after diffing one representative from each cluster against
  upstream's corresponding `expectedv2.ts` to verify shape parity.

- **R6: `$$_$$` collision.** Upstream uses the exact literal
  `$$_$$`. Users could (theoretically) write that in their code.
  Upstream relies on double-dollar being reserved. Our `__svn_*`
  prefix discipline suggests `__svn_$$` would be safer, but that
  loses the cross-checkable pattern. Stick with `$$_$$` and add a
  comment near emit that names it as upstream-equivalent.

### 5.2 Kill criteria

Abandon the port and revert to the deferred state if any of:

- **K1**: Stage 4 moves >80 snapshots (>40% of the svelte2tsx
  corpus) — indicates the shape diverges more than a mechanical
  port.
- **K2**: After Stage 5, layerchart doesn't hit 26 errors. A miss
  here means the scope resolver doesn't match upstream's semantics,
  and we ship a leaky type channel that's worse than the
  `let X: any` baseline.
- **K3**: Datagrid regresses by >10 errors. Datagrid's 43-file
  `let:` surface is the canary — if it goes red, the cost of
  running down the cause outweighs the +4-error win on Sankey.
- **K4**: The shim's `__svn_instanceOf` overload set proves
  unrepresentable (e.g. requires introducing a `typeof import`
  chain that breaks non-TS overlays). Fall back to emitting
  `$$slot_def: any` end-to-end with no typed channel. That
  gives us the right EMIT shape without the type-safety win,
  which is arguably worse than the status quo.

If killed, document the specific failure in
`notes/DEFERRED.md` under a new section "Slot resolver port
attempt" with the stage that blocked + the evidence.

---

## 6. Files touched (projection)

New:
- `crates/analyze/src/template_scope.rs` (~200 LOC)
- `crates/analyze/src/slot_resolver.rs` (~300 LOC)
- `design/slot_handler/fixtures/01-basic-let/` (7 fixtures total)

Modified:
- `crates/analyze/src/template_walker.rs` — thread scope through
  walk; add `slot_defs` to `TemplateSummary`; add `LetBindingSite`
  to `ComponentInstantiation`.
- `crates/analyze/src/lib.rs` — re-export new modules.
- `crates/emit/src/lib.rs` — sites `:1598`, `:1611`, `:2533-2567`,
  `:4952-4964`, `:5024-5088`. New shim declarations in whichever
  function emits the intrinsic header (grep for `__SvnInstance`).
- `crates/analyze/src/void_refs.rs` — register `$$_$$` as a shared
  name.

Snapshots (mechanical, UPDATE_SNAPSHOTS=1):
- `crates/cli/tests/emit_snapshots/htmlx2jsx/*-slot*/`
- `crates/cli/tests/emit_snapshots/svelte2tsx/*slot*/`
- Any bug fixture touching slots.

No changes:
- `crates/parser/*` — AST is complete.
- `crates/typecheck/*` — mapper stays; only emit shapes change.
- `crates/core/*` — TsConfig unaffected.

---

## 7. What to do if you're implementing this

Read `CLAUDE.md` first (Rules #1, #3, #8 especially), then:

1. Run `node scripts/diff-emit.mjs bench/layerchart/packages/layerchart/src/lib/components/Chart.svelte --upstream` and confirm the tail matches the shape quoted in §1.7 above. If not, the upstream submodule has moved and this plan needs a re-verify pass.
2. Write the Stage 0 fixtures. Commit before proceeding.
3. Implement Stage 1 in isolation. Run unit tests. Commit.
4. Continue stage-by-stage. Don't skip ahead — each stage's kill
   criterion is real.
5. After Stage 5, re-run the full bench fleet. Update
   `notes/OPEN.md` with the new counts.

If any stage blocks more than a day, escalate via a dedicated
OPEN.md entry with the specific divergence and the next debugging
step. The CLAUDE.md protocol "diff the real upstream artifact" is
non-negotiable — don't theorize about TS behavior without verifying
against an upstream overlay.
