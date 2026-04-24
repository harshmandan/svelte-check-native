# Phase G — Svelte 4 syntax compatibility

**Status:** design, not yet implemented.
**Target release:** v0.2.
**Key constraint:** every Svelte-4-specific code path lives in isolation so a future Svelte-4-is-officially-dead release can delete it cleanly — no refactor archaeology, no hidden callsites.

## Motivation

v0.1 intentionally excluded Svelte 4 syntax (see CLAUDE.md scope section). Real-world bench data now justifies reopening the scope: production codebases are rarely pure Svelte 5 during the migration window. Common shapes still in the wild:

- `export let foo` — **already supported** (v0.1).
- `export { name as alias }` — **already supported** (v0.1.1+).
- `<slot>`, `<slot name="foo">`, `<slot a={b}>` — **not yet**.
- `on:event` directives on elements and components — **not yet**.
- `$: x = expr` / `$: { … }` reactive statements — **not yet**.
- `interface $$Props` / `$$Events` / `$$Slots` declarations — **partially**; declared names are lifted but not fed into the emitted Component signature.

Every leaf Svelte 4 component in a mid-migration codebase cascades "missing prop", "no overload", or "unknown event" type errors into every Svelte 5 host that imports it. Supporting these four patterns closes the gap.

## Scope decision matrix

| Pattern | Action | Rationale |
|---|---|---|
| `<slot>` / named / with props | Translate to Svelte 5 snippet shape | Slots map 1:1 to snippets at the type level; one abstraction in our emit, not two. |
| `on:event` directive | Translate to Svelte 5 `onevent` prop | Already how Svelte 5 handles events; just a rename at emit time. |
| `$: x = expr` (declaration) | Translate to `let x = $derived(expr)` | Type-level semantics match exactly. |
| `$: x = expr` (re-assign) | Drop the `$:` label | Assignment already typed; the label is Svelte-4 bookkeeping. |
| `$: { stmt }` / `$: stmt` | Wrap in `() => { $: { … } }` | Mirrors upstream svelte2tsx — a never-called arrow that parses as legal TS. |
| `interface $$Events` | Feed into emitted `Component<Props, Events, Bindings>` | Already detected by analyzer, not yet consumed by emit. |
| `interface $$Props` | Use as Props type when no `$props()` call | Replaces our fallback synthesis for Svelte-4 components. |
| `interface $$Slots` | Use as slots record when no snippet form exists | Completes the triple. |
| `$$Generic` / `$$RestProps` | **Out of scope** | Fringe uses; can baseline. |
| `svelte:component` / `svelte:element` | **Out of scope in Phase G** | Separate effort; both exist in Svelte 5 too and want unified handling. |

## Architectural constraint — clean removal

Every file, function, test, and callsite introduced for Svelte 4 support must carry a visible marker so a future `grep svelte4` finds every trace.

**Organization:**

```
crates/analyze/src/svelte4/
  mod.rs              # pub use from {slot, on_directive, reactive, $$_interfaces}
  slot.rs             # detects <slot>, extracts name + prop bindings
  on_directive.rs     # detects on:event attributes on elements + components
  reactive.rs         # classifies $: forms (decl, reassign, block, statement)
  $$_interfaces.rs    # detects interface $$Props / $$Events / $$Slots

crates/emit/src/svelte4/
  mod.rs
  slot.rs             # emits __svn_slot_check(...) or $children?.(...) shape
  on_directive.rs     # maps on:event → onevent in the props object literal
  reactive.rs         # rewrites $: declarations → $derived, statements → $effect
  $$_interfaces.rs    # feeds $$Events / $$Slots into Component<...> signature

crates/typecheck/src/svelte_shims_svelte4.d.ts
  # __sveltets_2_* helpers used only by Svelte-4 emit paths; gated by
  # an emit-side "contains_svelte4_features" flag so Svelte-5-only
  # projects don't pay the shim cost.
```

**Callsites:** the main emit/analyze orchestrators call into the svelte4 modules at explicit decision points. Each call has a `// SVELTE-4-COMPAT` comment:

```rust
// SVELTE-4-COMPAT: rewrite $: x = expr → let x = $derived(expr) before
// split_imports so downstream passes see the Svelte-5 shape only.
let content = svelte4::reactive::rewrite(content, lang);
```

**Tests:** one integration fixture per pattern under `fixtures/svelte4/<pattern>/`. Skipped in Svelte-5-only runs via `cfg(feature = "svelte4_compat")` — but on by default. A `features = []` build compiles a clean binary without any Svelte 4 code in the produced artifact.

**Removal playbook (future):**

1. `cargo build --no-default-features` — binary without Svelte 4 support, still type-checks clean.
2. `rm -rf crates/*/src/svelte4/ fixtures/svelte4/ crates/typecheck/src/svelte_shims_svelte4.d.ts`.
3. `grep -rn "// SVELTE-4-COMPAT" crates/` → delete each callsite and the surrounding dispatch.
4. `grep -rn "svelte4" crates/` → assert empty (Cargo.toml feature-flag removal triggers any remainders).
5. `cargo fmt && cargo test --workspace`.

Total cleanup: <30 minutes of mechanical work, no human judgement required. That's the acceptance criterion for "cleanly removable."

## Per-pattern design

### 1. `on:event` → `onevent` prop rewrite

**Detection (analyze).** Walk every element/component attribute. An attribute whose name matches `/^on:[a-z][a-z0-9]*$/` and has a value (expression or bare) is an event directive.

**Emit transformation.** Produce the same object-literal key upstream's Svelte 5 host would have consumed:

```svelte
<!-- input -->
<button on:click={handleClick}>Go</button>
<Foo on:custom={handleCustom} />
```

```ts
// emit (unchanged element path)
{ svelteHTML.createElement("button", { "onclick": handleClick, … }); }

// emit (component — reuses the existing __svn_ensure_component shape)
{ const __svn_C_0 = __svn_ensure_component(Foo);
  new __svn_C_0({ target: __svn_any(), props: { oncustom: handleCustom } });
}
```

For DOM elements, tsgo picks up `onclick` from svelte-jsx's element typings (already in the shim). For components, the event becomes a prop lookup on the Props type; if the component declares `interface $$Events { custom: CustomEvent<…> }`, we emit it as part of `Component<Props, Events>` (see `$$_interfaces`) so `oncustom` resolves.

**Removal:** delete `on_directive.rs`; the main attribute walker falls back to literal-pass-through, any residual `on:*` attributes become unknown prop diagnostics.

### 2. `<slot>` → snippet shape

**Detection (analyze).** Walk the template. For each `<slot>` element:
- Name: `name="foo"` attribute → `"foo"`; missing → `"default"`.
- Props: all other attributes become a `{key: value}` record.
- Fallback: children as a snippet body (rarely typed; drop into the else branch).

**Emit transformation.** A slot's type contract:

```svelte
<slot a={b}>fallback</slot>
```

is equivalent to Svelte 5's:

```svelte
{@render children?.({ a: b })}
```

And the component's Props type gains `children?: Snippet<[{ a: typeof b }]>`.

For named slots, the Props type gains `<name>?: Snippet<[{ … }]>` per name, and the template body calls `props.<name>?.({ … })`.

**Why translate instead of inventing new helpers.** Our existing snippet emit path already handles `Snippet<[…]>` typing, destructuring, and scope flow. Reusing it means zero new compiler-side behaviour — we just generate Svelte-5-shaped emit from Svelte-4 input.

**Callsite:** template walker, right after recognizing the element name:

```rust
if name == "slot" {
    // SVELTE-4-COMPAT
    if let Some(shape) = svelte4::slot::recognize(element) {
        emit_snippet_call(shape);
        continue;
    }
}
```

**Removal:** delete `slot.rs`; the branch above becomes unreachable; remove the callsite. `<slot>` elements then emit as unknown HTML tags (harmless — tsgo reports unknown-element diagnostics, which is correct for a Svelte-5-only tool).

### 3. `$:` reactive statements

Three sub-shapes, decided at the AST level in analyze:

| Shape | Example | Emit |
|---|---|---|
| Declaration | `$: b = count * 2` | `let b = $derived(count * 2)` |
| Re-assignment | `$: count += step` (where `count` declared earlier) | `count += step;` (drop `$:` label) |
| Statement/block | `$: console.log(count)` / `$: { … }` | Wrap in `() => { $: … }` (matches upstream) |

**Why $derived and not upstream's `__sveltets_2_invalidate`?** Our shim is a Svelte 5 world. `$derived<T>(expr: T): T` already exists in our runes ambient. Mapping to it avoids importing an upstream helper and keeps the emit closer to what users would write in Svelte 5. Type-level semantics are identical.

**The block/statement wrapper is a svelte2tsx trick:**

```ts
() => { $: { console.log(a + 1); } }
```

`$: X` is a legal label-prefixed statement in JavaScript (where `$` is the label name). TypeScript type-checks the body but never invokes the arrow, so runtime semantics are irrelevant. Upstream uses this pattern; we'll match it verbatim.

**Callsite:** one emit-level rewrite pass in `crates/emit/src/svelte4/reactive.rs::rewrite(content)`, run after `state_nullish_rewrite` but before `split_imports`. Every `$: …` statement either gets rewritten in-place (declaration) or wrapped (statement/block).

**Removal:** delete the module; remove the `// SVELTE-4-COMPAT` rewrite call in `crates/emit/src/lib.rs`; any residual `$: …` in the user's script body flows through unchanged and tsgo reports "Labels used with `$` convention" or similar.

### 4. `$$Props` / `$$Events` / `$$Slots` interfaces

**Detection (analyze).** After parsing the instance script, look for top-level `interface $$Props` / `$$Events` / `$$Slots`. Lift them same way we lift user types (into `hoisted` so they're visible at module scope).

**Emit integration.** The existing default-export emission takes Props from `find_props_type_source`. Extend that to also check for `$$Props` when no `$props()` call exists. For `$$Events` and `$$Slots`, feed them as additional type parameters into the emitted `Component<Props, Events, Bindings>`.

**Changes:**
- `crates/analyze/src/props.rs::find_props_type_source` — add an "if no `$props()` call, look for `$$Props`" fallback (one branch).
- `crates/emit/src/lib.rs` default-export synthesis — when `$$Events` / `$$Slots` are present, expand the emitted `Component<P>` to `Component<P, E, B>`.

**Callsite marker:** `// SVELTE-4-COMPAT` on the `$$Props` / `$$Events` / `$$Slots` detection lines. Removal flips them back to "always use `$props()`" behavior.

## Ordering + validation

Per CLAUDE.md architectural rule 8 (every new emit shape is tsgo-validated on a hand-written fixture before implementation):

1. `design/phase_g/fixture/` gets a hand-written minimal `.ts` file per pattern + one deliberately-broken companion file each, with a `tsconfig.json` that invokes tsgo directly. The fixture's clean `.ts` must produce zero errors; the broken `.ts` must produce exactly the expected error list. This fixture lives in-tree and runs as a pre-commit check for the whole Phase G branch.

2. Once each pattern's fixture gates green, implement the analyzer + emit rules in the corresponding `svelte4/` module.

3. Integration fixtures under `fixtures/svelte4/<pattern>/` run against the full pipeline.

4. Parity check: the ported upstream `<slot>` / `on:` / `$:` samples from `language-tools/packages/svelte2tsx/test/svelte2tsx/samples/` should all hit zero errors in our pipeline. This is our v0.2 ship gate.

## Estimated work

Per pattern:

- `on:event` rewrite: ~0.5 day. Localized attribute-name transform.
- `$:` reactive: ~1 day. AST-level rewrite with sub-shape dispatch; `$derived` / `$effect` mapping is straightforward.
- `<slot>` → snippet: ~1-2 days. Template walker integration; named-vs-default dispatch; prop-binding extraction.
- `$$Props/Events/Slots`: ~0.5 day. Analyzer extension + emit plumbing.

Total: ~1 work-week assuming no unforeseen interactions with existing hoisting / declaration-stub logic.

## Out-of-band observations from bench data

- **A component-lib bench** (9 bench errors remaining) has two `$state<Promise<T>>` patterns we just fixed in 0.1.2; the rest are genuine bits-ui / tsgo union-complexity limits, not Svelte-4-pattern-related.
- **A UI-lib bench** (8 bench errors) are all sibling-collision barrel-re-export errors — Svelte 5 codebase, not Svelte 4.
- **A desktop-app bench and a PWA bench** (1 error each) also Svelte 5.
- **A reader-app bench** (270 errors) hasn't been investigated; likely Svelte 4 mixed codebase — Phase G's blast-radius sample.

Phase G success criteria on the reader-app bench specifically: unblock at least 75% of the 270 errors.

## Non-goals

- Rewriting user source files on disk. All Svelte-4 rewrites are overlay-only.
- Supporting every Svelte-4 pattern. `$$Generic`, `$$RestProps`, accessor syntax, `<svelte:self>` in Svelte-4 semantics — deferred or baselined.
- Automatically migrating user code from Svelte 4 to 5. Tooling for that exists elsewhere.

## Risks

- **`<slot>` with slot-props interacting with snippets.** Mixed Svelte-4-slot + Svelte-5-snippet usage in the same component is technically possible; our translation should degrade gracefully (prefer snippet when both present).
- **Reactive block variable-capture semantics.** `$: { a = b; }` where `a` is declared earlier is a reactive assignment, not a declaration. Our rewrite must not shadow.
- **`$$Events` consumed by both upstream shim helper and our helper.** If the user has `interface $$Events` at top level, our `__svn_ensure_component` may already pick it up via the `Component<Props, Events>` machinery; double-wrapping is harmless but we should pick one path.

## Open questions (resolved during implementation)

- Whether `$: a = b` assignments to previously-declared variables should fire the same `state_referenced_locally` compiler warning as Svelte 5's runic equivalent. Likely yes for parity; confirm against upstream behavior.
- Whether `<slot>` fallback content needs type-checking. Upstream doesn't seem to; we can follow.
- How `bind:group` interacts with the `on:change` directive on inputs. Probably orthogonal.
