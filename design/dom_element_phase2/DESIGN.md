# DOM element emit Phase 2 design

Branch: `crates/emit/src/lib.rs::emit_dom_element_open` + the
existing Node::Element / Node::SvelteElement arms.
Upstream spec: same files as Phase 1 (Element.ts + Attribute.ts)
plus the directive-specific files — `Class.ts`, `StyleDirective.ts`,
`Spread.ts`.

## What Phase 2 adds

Phase 1 covered regular HTML elements with Plain / Expression /
Shorthand attrs. Phase 2 adds:

1. **`class:foo={cond}` + `class:foo` (shorthand).** Value expression
   (or just the name for shorthand) emitted as a bare void statement
   after the createElement call; not in the attrs object.
2. **`style:prop={value}` + `style:color` (shorthand).** Value
   wrapped in `__svn_ensure_type(String, Number, value)` after the
   createElement call; validates against `string | number | null |
   undefined`.
3. **`svelte:body` / `svelte:head` / `svelte:window` /
   `svelte:document` / `svelte:options` / `svelte:fragment`.** Colon
   stays verbatim in the tag name literal — the svelteHTML
   IntrinsicElements catalog has these as string keys.
4. **`svelte:element this={tag}`.** First arg to createElement is
   the tag expression directly (not a string literal), so TS checks
   it against IntrinsicElements keys.
5. **Spread `{...props}`.** Emitted as a spread element inside the
   attrs object: `{ svelteHTML.createElement("tag", { ...props, }); }`.

## Fixture — what it locks

`design/dom_element_phase2/fixture/` — tsgo-validated 2026-04-24.

- `01_class_directive.svelte.svn.ts` — class directive shorthand +
  expression forms emit the value as a bare statement post-create.
  0 errors.
- `02_style_directive.svelte.svn.ts` — style directive values wrapped
  in `__svn_ensure_type(String, Number, …)`. 0 errors.
- `03_svelte_body.svelte.svn.ts` — svelte:body/head/window/document
  use the colon-name literal. 0 errors.
- `04_svelte_element.svelte.svn.ts` — svelte:element this={tag}
  passes the tag expression as the first createElement arg. 0 errors.
- `05_spread.svelte.svn.ts` — spread attrs emit as `...props`. 0 errors.
- `Errors.svn.ts` — locks 2 diagnostics:
  - TS2304 — class:missing references undeclared name.
  - TS2345 — style:color={booleanValue} argument-type mismatch.

## Rust port plan

All changes sit in `crates/emit/src/lib.rs`:

### 1. Add `__svn_ensure_type` to the shim

`crates/typecheck/src/svelte_shims_core.d.ts` gains a 2-overload
`__svn_ensure_type<T>` helper mirroring upstream's
`__sveltets_2_ensureType`. Accepts 1 or 2 constructor types plus
a value; returns void.

### 2. Extend `emit_dom_element_open` → two-pass

First pass: emit attrs that go INSIDE the createElement call (Plain,
Expression, Shorthand, Spread). Second pass (emitted AFTER the
createElement close) — emit directive-value statements (class: and
style:) inside the same scoped block, before children. Both passes
run before `emit_children_with_let_bindings`.

### 3. Node::SvelteElement branch

`Node::SvelteElement` arm gets its own DOM-emit path. For
`svelte:element`, read `this={tag}` expression and pass it as the
first arg. For `svelte:body/head/window/document/options/fragment`,
pass the literal string with colon.

### 4. Spread attrs

`Attribute::Spread` pushes `...<expr>,` into the attrs object.

## A/B gate

Phase 2 lands under the same `SVN_DOM_ELEMENT_EMIT` default-on gate
(Phase 1 flipped default-on in commit 85bcaa1). Verify fresh A/B on
bench suite, expect additional diagnostics closing the CMS bench's
under-fire gap further.

## Non-goals for Phase 2

- Transition / In / Out / Animate directives — emit nothing
  attribute-side; they type-check through their own helpers already.
- Let directives (`let:x`) — already handled by
  `emit_children_with_let_bindings` (slot-let scope).
- `{@attach}` / attachment tags — Svelte 5.30+ feature, separate pass.
