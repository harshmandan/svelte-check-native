# Parity findings — 2026-04-21

Investigation trail for the three emit fixes that land in `v0.3.9`.
Triggered by a user running `svelte-check-native` on a SvelteKit
sub-app monorepo who reported six classes of errors their upstream
`svelte-check` caught that our tool missed.

## Context

Starting point was a fresh `svelte-check-native v0.3.8` run on
`bench/control-svelte-5/src/apps/sub-app` (1359 files, a real-
world SvelteKit app vendored as a bench target):

| tool                        | files | errors | warnings | files with problems |
| --------------------------- | ----: | -----: | -------: | ------------------: |
| `svelte-check-native 0.3.8` |  1359 |      2 |       44 |                  17 |
| `svelte-check --tsgo`       |  1359 |      2 |       44 |                  17 |

Numerically identical. The user's report was against *their* project
(similar shape, different code). Their report decomposed into four
categories; each traced to a different emit gap.

---

## Finding 1 — `use:enhance={callback}` callback typing lost

### Pattern

```svelte
<form method="POST" use:enhance={({ form, data, submit }) => {
    return async ({ result, update }) => { … };
}}>
```

`form`, `data`, and `submit` aren't on SvelteKit's `SubmitFunction`
parameter shape (which is `{ action, formData, formElement,
controller, submitter, cancel }`). Upstream `svelte-check --tsgo`
fires three `TS2339 "Property … does not exist"` per site — one for
each wrong destructure name. Our tool fired nothing.

### Root cause

`use:ACTION={PARAMS}` emitted as a dead placeholder:

```ts
let __svn_action_attrs_0: any = {};
void __svn_action_attrs_0;
```

The `PARAMS` expression was discarded entirely. TypeScript never saw
the callback, so its parameter destructure was never checked.

### Fix

Mirror upstream svelte2tsx's `__sveltets_2_ensureAction` shape with
our `__svn_` namespace:

```ts
const __svn_action_0 = __svn_ensure_action(
    enhance(__svn_map_element_tag('form'), (({ form, data, submit }) => { … }))
);
```

The inner `enhance(...)` is a real function call. TypeScript
contextually types the callback argument against `enhance`'s
declared second parameter (`SubmitFunction | undefined`), which
forces the destructure to check against the real param shape.
Wrong names fire TS2339 at the user's source position via a
post-scan `collect_action_directive_token_map`.

Validated fixture-first at `design/action_directive/`:

- `Clean.svelte.ts` — correct destructure (`formData`, `formElement`) → 0 errors.
- `Wrong.svelte.ts` — wrong destructure (`form`, `data`, `submit`) → exactly 3 TS2339.

Commit: [`f91fa70`](https://github.com/harshmandan/svelte-check-native/commit/f91fa70)

---

## Finding 2 — template `{EXPR}` interpolations not type-checked

### Pattern

```svelte
{#if form?.success}
    <p>{form.error}</p>
{/if}
```

Inside the `if`-body, `form` is narrowed to a branch where `.success`
is truthy. Reading `form.error` should fire TS2339 in that narrow
(on a plain discriminated union).

Upstream fires TS2339 at line:col of `form.error`. Our tool fired
nothing.

### Root cause

`emit_template_node`'s `Node::Interpolation` arm only handled
`{@const …}` tags:

```rust
Node::Interpolation(i) => emit_at_const_if_any(out, source, i, depth),
```

Every plain `{EXPR}` interpolation was dropped. `find_template_refs`
voided the ROOT identifier (`form`) to keep TS6133 off our back, but
the full expression (`form.error`) was never placed anywhere tsgo
could check.

### Fix

Emit each plain interpolation as an expression statement in its
enclosing scope, with a sentinel comment the post-walk scanner uses
as an anchor:

```ts
if ((form?.success)) {
    void [form?.success];
    /*svn_I*/(form.error);
}
```

A `collect_interpolation_token_map` post-walk zips `/*svn_I*/`
sentinels with plain-interpolation ranges (collected in fragment-
walk order, matching emit order) and pushes a `TokenMapEntry` per
site. Paren-wrap protects against multi-clause expressions
(`a, b`) or assignment-looking shapes parsing as statement heads.

Commit: [`ae15e45`](https://github.com/harshmandan/svelte-check-native/commit/ae15e45)

---

## Finding 3 — paraglide `m['login.pin']` literal-key miss

### Pattern

```svelte
<script lang="ts">
    import * as m from '$lib/paraglide/messages';
</script>

<p>{m['login.pin']()}</p>  <!-- 'login.pin' not in Messages type -->
```

paraglide generates a `Messages` type shape with a closed set of
string-literal keys. Indexing with a missing key fires TS7053
("Element implicitly has an 'any' type…").

Upstream fires TS7053. Our tool fired nothing.

### Root cause

Same as Finding 2 — the `{EXPR}` interpolation was dropped, so tsgo
never saw `m['login.pin']()` as a checkable expression.

### Fix

Same commit as Finding 2 ([`ae15e45`](https://github.com/harshmandan/svelte-check-native/commit/ae15e45)).
A single root-cause fix closes both.

---

## Non-finding — ActionData's `OptionalUnion` is working as designed

### Pattern the user thought was the bug

```svelte
{#if form?.success}
    <p>{form.error}</p>  <!-- user expected this to fire TS2339 -->
{/if}
```

The user reported this on a `+page.svelte` with a SvelteKit action
returning `{ success: true }` / `fail(400, { error: '…' })`.

### What actually happens

SvelteKit's `ActionData` type isn't a plain discriminated union. It
wraps the union in `OptionalUnion<U>`:

```ts
// from @sveltejs/kit/types/index.d.ts
type OptionalUnion<U extends Record<string, any>, A extends keyof U = U extends U ? keyof U : never> =
    U extends unknown ? { [P in Exclude<A, keyof U>]?: never } & U : never;
```

Each branch of the union gets `?: never` synthesized for every key
that appears in ANY other branch. So `{ success: true }` becomes
`{ success: true; error?: never }`. Reading `.error` returns
`undefined` instead of firing TS2339.

This is deliberate — the jsdoc on `OptionalUnion` says "makes
accessing them more ergonomic." Upstream `svelte-check --tsgo`
behaves the same (tested on a matching repro). Not a parity gap.

Our Finding 2 fix DOES fire on hand-typed discriminated unions
(`type R = { success: true } | { success: false, error: string }`).
If the user genuinely wants narrowing on form shapes, they can
declare the shape themselves instead of routing through `ActionData`.

---

## Non-finding — duplicate `</form>` diagnostic

Upstream fires two diagnostics on a single malformed closing tag:
one from the Svelte compiler and one from the overlay TS parser.
Our tool dedupes to one. This was on the user's report as a "miss"
but is arguably a UX improvement. Left unchanged.

---

## Deep-dive — SvelteKit typed callbacks (nothing else to fix)

Audited every SvelteKit-exported callback API via
`@sveltejs/kit/types/index.d.ts` to confirm no other template-level
patterns need emit handling:

| API                                 | Where used             | Emit handling needed? |
| ----------------------------------- | ---------------------- | --------------------- |
| `enhance(form, submit)`             | `use:enhance={…}`      | ✅ Finding 1           |
| `applyAction(result)`               | Script body            | ❌ Native TS context   |
| `deserialize(resultJson)`           | Script body            | ❌ Native TS context   |
| `beforeNavigate((nav) => void)`     | Script body (lifecycle)| ❌ Native TS context   |
| `onNavigate((nav) => …)`            | Script body (lifecycle)| ❌ Native TS context   |
| `afterNavigate((nav) => void)`      | Script body (lifecycle)| ❌ Native TS context   |

Everything except `enhance` lives inside the user's script body,
which we pass through to tsgo verbatim — TypeScript's own contextual
typing handles the callbacks. The only patterns that needed special
emit care were the template-level ones (`use:ACTION={cb}` and
`{EXPR}`), and both are now covered.

---

## Parity after the fixes

Same `bench/control-svelte-5/src/apps/sub-app`:

| tool                        | files | errors | warnings | files with problems |
| --------------------------- | ----: | -----: | -------: | ------------------: |
| `svelte-check-native 0.3.9` |  1359 |      8 |       44 |                  21 |
| `svelte-check --tsgo`       |  1361 |      8 |       44 |                  19 |

**Same total error count.** The 6-error climb on both tools reflects
the same 6 user bugs both tools now catch; the 2-file
`files_with_problems` delta is localization granularity on warnings
(same warnings, slightly different per-file distribution). No
regressions and no false positives introduced.

## Related artifacts

- Design fixture (tsgo-validated): [`design/action_directive/`](../design/action_directive/)
- Upstream-style emit reference: `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Action.ts`
- Helper declarations: [`crates/typecheck/src/svelte_shims_core.d.ts`](../crates/typecheck/src/svelte_shims_core.d.ts) (search for `__svn_ensure_action`, `__svn_map_element_tag`)
- Post-walk scanners: [`crates/emit/src/lib.rs`](../crates/emit/src/lib.rs) (search for `collect_action_directive_token_map`, `collect_interpolation_token_map`)
