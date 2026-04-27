# Gap C — Assignment-narrowing inside template-check (validated 2026-04-27)

## Problem

`threlte/theatre` over-fires +2 errors:

> `Project.svelte:38:25 "Type 'IProject | undefined' is not assignable to type 'IProject'."`
> `Sheet.svelte:37:23 "Type 'ISheet | undefined' is not assignable to type 'ISheet'."`

User pattern (Project.svelte simplified):

```ts
interface Props {
  project?: IProject;
  children?: Snippet<[{ project: IProject }]>;
}
let { project = $bindable(), children }: Props = $props();
project = pool.get(name) ?? createProject(name);
// ... template references children?.({ project })
```

After the `?? createProject(name)` assignment, TS should narrow
`project` from `IProject | undefined` to `IProject`. We didn't.

## Root cause

Our template-check wrapper used to be:

```ts
async function __svn_tpl_check() {
  // ... project ...
}
```

`async function NAME()` is a **function declaration** — TS hoists it
to the top of its scope. Hoisting collapses the assignment-narrowed
type back to the declared union, so `project` inside the function
body stays `IProject | undefined` regardless of the assignment that
happened lexically before it.

Upstream emits the same shape but as an **arrow expression
statement**:

```ts
async () => {
  // ... project ...
};
```

Arrow expressions are NOT hoisted. TS's control-flow analysis
treats them like any other expression at the position they appear,
so the narrowed type at that point flows into the arrow body.

## Validated fix shape

`design/gap_c_assignment_narrowing/fixture/src/repro.ts` shows three
variants side-by-side under `tsgo --pretty false -p tsconfig.json`:

- Variant A (named function decl) — fails TS2345.
- Variant B (arrow expression statement) — clean.
- Variant C (arrow IIFE inside `void (...)`) — clean.

Switching to Variant B closes Gap C.

## Cascading effect — Gap C cascade

Switching to the arrow form ALSO exposed an unrelated emit gap:

**cobalt/web** had been silently relying on the function-declaration
hoisting to de-narrow a `let processor: Processor = "stripe"` binding.
With the new arrow form, TS narrows `processor` to its initial
literal `"stripe"` and `processor === "liberapay"` fires TS2367
("no overlap"). Upstream is clean here because their overlay emits
`<button on:click={(e) => (processor = "stripe")}>` as
`"on:click": (e) => (processor = "stripe")` in the createElement
attribute literal — TS sees the reassignment in the closure, treats
`processor` as potentially mutated, and de-narrows.

**Our DOM emit silently dropped Svelte-4 `on:event={...}`
directives.** Pre-Gap-C this didn't matter because hoisting
de-narrowed everything regardless. Post-Gap-C the gap surfaces.

Fix: emit `"on:NAME": (handler)` keys in the createElement
attribute literal (matches upstream's `EventHandler.ts:24-32` for
the DOM-element branch).

That introduces a SECOND cascade: the user idiom `<el on:click={fn}
on:click>` (handle + forward) produces duplicate `"on:click"` keys
in the literal — both ours and upstream emit duplicates verbatim,
both fire TS1117 ("multiple properties same name").

Upstream filters TS1117 (and TS2300) when the diagnostic position
is on an Element attribute name, via `isAttributeName(node,
'Element') || isEventHandler(node, 'Element')` AST checks
(`DiagnosticsProvider.ts:360-374`). We don't carry the Svelte AST
to the diagnostic mapper, so we filter via overlay byte scan: any
diagnostic position landing on a quoted `"<key>"` followed by `:`
(an attribute key in a createElement literal) gets its TS1117/TS2300
dropped.

## Implementation

Three coordinated changes:

1. `crates/emit/src/render_function.rs::emit_template_check_fn` —
   emit `;(async () => { ... });` instead of `async function
   __svn_tpl_check() { ... }`. Drop the trailing `void
   __svn_tpl_check;`.
2. `crates/emit/src/nodes/element.rs::emit_dom_element_open` — add
   a branch for `Directive::On` that calls
   `emit_dom_event_handler` to emit `"on:NAME": (handler)` (or
   `: undefined` for bubbling) inside the createElement literal.
3. `crates/typecheck/src/lib.rs::map_diagnostic` — filter TS1117
   and TS2300 when the overlay byte scan detects a
   `"key":` attribute pattern.

Plus tests + fixtures:
- `fixtures/bugs/80-assignment-narrowing-template-check` — locks
  the narrowing closure
- `fixtures/bugs/81-on-event-dom-emission` — locks the on:event
  emission and duplicate-key filter
- Updated unit tests in `crates/emit/src/lib.rs` and
  `crates/analyze/src/template_walker.rs` to assert the new
  arrow-expression shape

## Bench impact

| Bench | Before | After |
| :--- | :--- | :--- |
| threlte/theatre | 9E (Gap B+C+misc) | 4E byte-perfect on errors |
| threlte/flex | 3E (Gap B) | 2E byte-perfect on errors |
| cobalt/web | 2E | 2E (held; Gap C cascade closed) |
| layerchart | 26E | 26E (held; TS1117 cascade closed) |

No regressions on the 11 byte-perfect benches.
