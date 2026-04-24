# DOM element emit design

Gate: `SVN_DOM_ELEMENT_EMIT=1`.
Branch: `crates/emit/src/lib.rs::emit_template_node` → `Node::Element(e)`.
Upstream spec: `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Element.ts`.

## Why this exists

Palacms under-fire gap 2026-04-24: we match upstream on CodeMirror
(both report 28 errors) but under-fire by 67 total across the
workspace. Root cause: we don't emit DOM elements. Upstream emits
each `<div …>` / `<button …>` / `<input …>` as

```ts
{ const $$_div = svelteHTML.createElement("div", { …attrs }); }
```

which binds every attribute value against the typed element slot
(pulled from `svelte/elements`' `SvelteHTMLElements[Property]`). The
whole attribute-type-check universe lives inside that call:
button-type literal unions, event-handler parameter typing, invalid
attribute names on `HTMLAttributes<T>`, action-callback arity. Our
template walker handles only `bind:*`, `use:*`, and plain `{expr}`
interpolations — the attribute cluster is invisible to tsgo.

## The shape

Reproduced from the real upstream overlay of a `Slider.svelte`
pattern on a Svelte-5 CMS-style bench (captured via
`svelte2tsx({ mode: 'ts' })`):

```ts
async () => {
  { svelteHTML.createElement("div", {});
    { svelteHTML.createElement("p", { "class": `label`, }); field.label; }
    { svelteHTML.createElement("div", { "class": `container`, });
      { svelteHTML.createElement("p", { "class": `value`, }); value; }
      { svelteHTML.createElement("input", {
          "oninput": ({ target }) => onchange({ [field.key]: { 0: { value: target.value } } }),
          "class": `input`,
          value,
          "type": `range`,
        });
      }
    }
  }
};
```

Each element opens a scoped block `{ … }`. The createElement call is
the first statement; children follow. Attribute shape:

| Svelte template            | Emit shape                                      |
| :------------------------- | :---------------------------------------------- |
| `class="label"`            | `"class": \`label\`,`                           |
| `class={variants}`         | `"class": variants,`                            |
| `{value}` (shorthand)      | `value,`                                        |
| `type={type}`              | `type: type,` (or shorthand `type,`)            |
| `onclick={(e) => …}`       | `onclick: (e) => …,`                            |
| `oninput={({target}) => …}`| `oninput: ({ target }) => …,`                   |
| `class:active={on}`        | (class directive — later phase)                 |
| `style:color={c}`          | (style directive — later phase)                 |
| `use:action={cb}`          | (action — separate `const $$action_0 = __sveltets_2_ensureAction(…)`) |
| `bind:value={x}`           | (already handled by `emit_element_bind_checks_inline`) |

Template literals (backticks) for string attribute values come from
upstream's span-remapping strategy: the surrounding quotes in the
Svelte source (`"label"`) become backticks so interpolations inside
attribute values (`class="foo-{cond ? 'a' : 'b'}"`) splice without
span rewrites. A bare backtick string with no `${…}` is
type-equivalent to its literal.

## Fixture — what it locks

`design/dom_element_emit/fixture/` — tsgo-validated on 2026-04-24.

- `01_plain_attr.svelte.svn.ts` — `<button type={t} disabled={d}>`
  with literal-union `type`. Clean case, 0 errors.
- `02_shorthand.svelte.svn.ts` — `<input {value}>` — shorthand expands
  to the same `name,` shape upstream uses. 0 errors.
- `03_event_handler.svelte.svn.ts` — `onclick: (e) => e.clientX`.
  Contextual `MouseEvent` typing resolves `e.clientX` and
  `e.currentTarget.disabled`. 0 errors.
- `04_oninput_destructure.svelte.svn.ts` — the Slider shape, but
  corrected: reach for `e.currentTarget.value` instead of
  destructuring `target`. 0 errors.
- `05_string_literal_attr.svelte.svn.ts` — `"class": \`label\`,`
  backtick form. 0 errors.
- `06_nested_elements.svelte.svn.ts` — scoped-block nesting across a
  4-level Slider.svelte shape. 0 errors.
- `Errors.svn.ts` — 4 deliberately-broken cases locking:
  - TS2322 button type literal union (save-button component shape).
  - TS2339 `EventTarget.value` (Slider destructure shape).
  - TS2322 input value boolean.
  - TS2353 unknown attr `onclick_outside` on HTMLButtonAttributes.

Command: `tsgo --pretty false -p design/dom_element_emit/fixture` →
exactly these 4 errors. Clean files produce zero.

The fixture's `svelte_html_shim.d.ts` is a trimmed model of the real
`svelteHTML` namespace — just enough for the test attributes. The
production overlay uses the user's real `svelte-jsx-v4.d.ts` (shipped
with Svelte) which declares the same namespace shape. Locking the
FIXTURE shape proves the emit will typecheck against the production
types too.

## Rust port plan

### Phase 1 — gate + single-element family

`SVN_DOM_ELEMENT_EMIT=1`. In
`crates/emit/src/lib.rs::emit_template_node`'s `Node::Element(e)`
arm, add a new `emit_dom_element_call(buf, source, e, depth)` that:

1. Opens a scoped block `{ `.
2. Writes `svelteHTML.createElement("<tag>", { `.
3. Iterates `e.attributes`:
   - Plain `name={expr}` / `name="literal"` → `"name": <expr or \`literal\`>,`
   - Shorthand `{expr}` → `<expr>,`
   - Skip `bind:*` / `use:*` (handled by existing branches).
   - Skip `class:*` / `style:*` (Phase 2).
4. Closes `});` and recurses into children (existing
   `emit_children_with_let_bindings`).
5. Closes the scoped block `}`.

Runs BEFORE the existing `emit_element_bind_checks_inline` +
`emit_use_directives_inline` calls — they stay as-is since the call
site types are orthogonal.

### Phase 2 — directives + svelte:*

Extend `emit_dom_element_call` to:
- `class:foo={cond}` — attribute expanded to an index-signature
  access; upstream emits `"class:foo": cond,`.
- `style:prop={v}` — same shape.
- `svelte:body` / `svelte:head` / `svelte:window` / `svelte:options`
  — emit `svelteHTML.createElement("svelte:body", { … })` (the colon
  survives).
- `svelte:element this={tag}` — emit
  `svelteHTML.createElement(tag, { … })` (expression as first arg).
- `<slot>` — already a separate path (`__sveltets_createSlot`).

### A/B validation

After Phase 1 lands:

```sh
SVN_DOM_ELEMENT_EMIT=1 ./target/release/svelte-check-native \
  --workspace $(pwd)/bench/<cms-bench> \
  --tsconfig $(pwd)/bench/<cms-bench>/tsconfig.json --output machine
```

Target: -40 errors on the CMS bench (closes save-button / slider /
image-field-options / page-type-form clusters). Must not regress
the Svelte-4 or Svelte-5 control benches (both at 0).

Phase 2 A/B one family at a time (class directive first, then style,
then svelte:*), each with its own under-flag run.

### Flip the gate

Once Phase 1 + 2 land and all three benches (CMS, Svelte-4
control, Svelte-5 control) verify at or below previous error
counts, default-on the gate. Add to CHANGELOG. Delete the
`SVN_DOM_ELEMENT_EMIT` env plumbing in a follow-up commit.

## Open questions (deferred)

- **`HTMLAttributes<T>` index signature.** Upstream's `svelteHTML`
  namespace uses strict shapes per intrinsic element, which means
  arbitrary data attributes (`data-testid`, `aria-label`, user actions
  as attributes) fire TS2353. Real-world code works because
  `svelte/elements`' `HTMLAttributes` includes `[key: \`data-\${string}\`]`
  and `[key: \`aria-\${string}\`]` index signatures. Phase 1 punts
  because the fixture locks only the strict shape; if a real
  bench fires new TS2353 on data attrs, we'll need to confirm the real
  `svelteHTML` handles them and adjust.
- **Computed attribute names** (`<button {...props}>`) — whether
  spread attributes emit as a named key or skip entirely. Needs its
  own fixture entry before implementation.
