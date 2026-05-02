# `design/` — emit-shape tsgo-validation fixtures

Per CLAUDE.md rule #8: any change to what the emit crate produces is
first expressed as hand-written TS, validated with tsgo, and only
then ported into Rust. Each subdirectory here is one such validation
— a "clean" fixture that should produce zero diagnostics, plus a
"broken" companion that should produce exactly the expected ones.

These are throwaway by design. Once the corresponding emit lands,
the fixture's purpose is served. Periodically prune dirs whose work
shipped long ago and is now well-settled — git history retains them.

## Current contents (HEAD 2026-05-02)

Kept around as recent references for active codepaths or as
documented charter exceptions. Older shipped fixtures (Phase A, all
Gap A-D, dom_element_emit, kit_route_types, slot_handler,
class_wrapper, etc.) were cleaned out during the post-v0.7.2 audit;
fetch from git history if you need them.

| Dir | What it validates | Shipped via |
|---|---|---|
| `gap_e_snippet_typing/` | Charter exception — ours more correct than upstream on snippet-receiver typing. WORKING AS EXPECTED. | n/a (charter) |
| `precise_event_typing/` | `<Child on:NAME />` typed event-name narrowing | R-Conv #12 (Cluster B) |
| `runes_void_emit/` | Runes void-emit selectivity for `$props()` | R-Conv #21 (V5 Phase 4) |
| `shim_infer_constraints/` | Inferred-constraint shim shape for `__svn_*` helpers | R-Conv #9 (Pre-Tier-1) |
| `slot_create_slot/` | `<slot>` synthesis via `__svn_create_create_slot<$$Slots>()` | R-Conv #20 B2 #3 |
| `slot_let_consumer_no_shadow/` | Slot-let consumer scope no-shadow | R-Conv #10 (D-iii) |
| `slot_let_no_implicit_children/` | Slot-let bypasses implicit-children synth | R-Conv #13 |
| `svelte5_bindings_postcheck/` | Svelte-5 `__svn_$$bindings()` literal check | R-Conv #19 (D-ii) |

## Workflow

```sh
# 1. Mirror upstream's emit shape verbatim (except `__sveltets_2_*` →
#    `__svn_*` and our shim namespace).
$EDITOR design/<topic>/clean.ts

# 2. Companion that should produce exactly the expected
#    diagnostics — same shape, semantics deliberately broken.
$EDITOR design/<topic>/broken.ts

# 3. tsconfig matching what our overlay sets — strict, isolatedModules,
#    skipLibCheck, lib: ES2022+DOM+DOM.Iterable.
$EDITOR design/<topic>/tsconfig.json

# 4. Validate. Clean must produce 0 diagnostics; broken must
#    produce EXACTLY the expected codes at the expected positions.
tsgo --noEmit -p design/<topic>/tsconfig.json
```

Only after both fixtures gate green does Rust implementation
begin. If the broken fixture doesn't fire the expected codes, the
theory is wrong and coding won't fix it.
