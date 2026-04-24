# Plan — unify default-export shape with `$$IsomorphicComponent`

**Status:** research complete, implementation not started.
**Goal:** close the 3 bench over-fires on component `bind:this` surfaced by 06003398 — Chart.svelte 421:20, Pala.svelte 146:15, datagrid +1. Make `new __svn_C(MyComp)({...})` produce an instance whose type matches whatever the user's `let x: MyComp` target resolves to.

**Non-goal:** changing the observable behavior on any already-passing case. This is a shape refactor that should be invisible on non-`bind:this` code.

Every upstream citation below refers to the pinned submodule at `language-tools/`.

---

## 1. Background — why the three fires exist

### 1.1 What upstream does

`language-tools/packages/svelte2tsx/src/svelte2tsx/addComponentExport.ts:168-179` emits, for every Svelte-5 component:

```ts
interface $$IsomorphicComponent {
    new <T>(options: ComponentConstructorOptions<Props & { children?: any }>):
        SvelteComponent<Props, Events, Slots> & { $$bindings?: Bindings } & Exports;
    <T>(internal: unknown, props: Props): Exports;
    z_$$bindings?: Bindings;
}
const Comp: $$IsomorphicComponent = null as any;
type Comp = InstanceType<typeof Comp>;
export default Comp;
```

Key property: **the TYPE alias is `InstanceType<typeof VALUE>`** — computed from the value's `new` signature. So `let x: Comp` resolves to exactly the shape `new Comp({...})` produces. Self-consistent by construction.

Upstream's single `__sveltets_2_ensureComponent<T>` helper (svelte-shims-v4.d.ts:224-251) has ONE conditional-return overload:

```ts
declare function __sveltets_2_ensureComponent<T extends …>(type: T): NonNullable<
    T extends ConstructorOfATypedSvelteComponent ? T
    : T extends Component<infer Props, infer Exports, infer Bindings>
        ? new (options: ComponentConstructorOptions<Props>) =>
            SvelteComponent<Props, Props['$$events'], Props['$$slots']>
              & Exports
              & { $$bindings: Bindings }
        : never>;
```

The `new` return's shape is **identical** to what `$$IsomorphicComponent`'s `new` returns. So `new (ensureComponent(X))({...})` instance and `let x: X` target are the SAME SHAPE.

### 1.2 What we do today (verified via real Chart.svelte overlay dump)

**Our emit has five dispatch branches** in `emit_default_export_declarations` (crates/emit/src/lib.rs:1747-2212):

- **JS branch** (:1789-1810). JSDoc `@typedef` wrapper. Not our problem.
- **`$$IsomorphicComponent` branch** (:2029-2096). Matches upstream shape; ALREADY IMPLEMENTED. Emits the interface, `const __svn_component_default: $$IsomorphicComponent`, and `type __svn_component_default<T> = InstanceType<...>`. Verified fires on Chart.svelte.
- **4-arm match** (:2098-2195), gated behind the iso branch:
  - Arm 1 `(Some(ty), Some(g)) if ty_safe_in_generic_scope` (:2099-2111) — function-sig value + separate SvelteComponent<P> type alias.
  - Arm 2 `(Some(ty), None)` (:2113-2123) — `Component<P>` value + separate `SvelteComponent<P>` type alias.
  - Arm 3 `(_, Some(g))` (:2125-2162) — fallback with generics.
  - Arm 4 default (:2164-2194) — `Component<Awaited<...>['props']>` value + `SvelteComponent<Awaited<...>['props']>` type alias.

**Crucial:** the iso-interface branch is gated on `use_class_wrapper && (has_export_let || is_runes_mode(doc)) && generics.is_some()` (:2029-2030). Many components don't qualify — **TransformContext.svelte goes through Arm 4 because it has NO generics**. Verified via overlay dump: `node_modules/.cache/svelte-check-native/svelte/src/lib/components/TransformContext.svelte.svn.ts:365-367` emits Arm 4's shape.

Our shim's `__svn_ensure_component` (crates/typecheck/src/svelte_shims_core.d.ts:472-502) has **5 overloads** that split by input type:

1. Constructor class — returns `C` passthrough.
2. `Component<P,E,S> & { __svn_events: E }` — typed-events marker, returns `new (...) => __SvnInstanceTyped<P, E>`.
3. `Component<P, any, any>` — returns `new (...) => __SvnInstance<P>`.
4. `(anchor, props: P) => any` — callable-function, returns `new (...) => __SvnInstance<P>`.
5. `unknown` fallback — returns `new (...) => __SvnInstance<any>`.

`__SvnInstance<P>` is `{ $$prop_def: P; $on(event: string, handler: (...args: any[]) => any): () => void }` (:519-522). `__SvnInstanceTyped<P, E>` is similar but with a narrow `$on<K extends keyof E>` (:547-553).

### 1.3 The specific Chart.svelte failure

`bench/layerchart/packages/layerchart/src/lib/components/Chart.svelte:421`:

```svelte
<TransformContext bind:this={transformContext} mode={...} {...transform} ondragstart ... />
```

Chart declares `let transformContext: TransformContext = undefined;` (line 318).

**Our overlay** (`Chart.svelte.svn.ts:460-464`):

```ts
const __svn_C_5d55 = __svn_ensure_component(TransformContext);
const __svn_inst_5d55 = new __svn_C_5d55({ target: __svn_any(), props: {mode, ...transform, ondragstart, ..., children: () => __svn_snippet_return()} });
transformContext = __svn_inst_5d55;
```

Substituting types:
- `TransformContext` value: `Component<Partial<TransformProps & PropsWiden>, Exports>` (from TransformContext's Arm 4 const declaration — :365).
- `__svn_ensure_component(TransformContext)`: overload 486-496 matches (TransformContext has no `new`, so overload 472 fails). Returns `new (options) => __SvnInstance<Partial<TransformProps & PropsWiden>>`.
- `__svn_inst_5d55`: `__SvnInstance<Partial<TransformProps & PropsWiden>>` = `{ $$prop_def: Partial<TransformProps & PropsWiden>; $on(...) }`. **Has exactly 2 fields.**
- `transformContext: TransformContext` target: type alias `TransformContext` from TransformContext's Arm 4 `type __svn_component_default` — resolves to `SvelteComponent<Partial<TransformProps & PropsWiden>> & Exports`. **Has ~12 SvelteComponent fields + Exports keys.**

TS2322: our 2-field `__SvnInstance<...>` is not assignable to the 12-field `SvelteComponent<...> & Exports` target.

**Upstream's shape** for the same bind-this assignment would be: instance is `SvelteComponent<Props, Events, Slots> & Exports & {$$bindings: B}`, target is `InstanceType<typeof TransformContext>` = the same shape. Match.

### 1.4 Why our earlier partial attempts didn't close it

1. **Broadening `__SvnInstance<P>` to intersect `Omit<SvelteComponent<P>, '$on'>`**: Fixes the SvelteComponent-field half, but target also requires `& Exports`. Our shim has no channel to thread Exports from the ensure-component input to the instance output.
2. **Routing LHS through TokenMapEntry**: Surfaces the real error at the user source. Good for fixture 68 (where error is legit), but these three bench cases are artifacts of the shim shape — surfacing them there was "honest but wrong."

---

## 2. Port sketch — three coordinated changes

### 2.1 Change #1 — default-export emit: unify all four arms into the `$$IsomorphicComponent` pattern

**Files:** `crates/emit/src/lib.rs` — `emit_default_export_declarations` (:1747-2212).

**What it becomes:** REPLACE the 4-arm match (:2098-2195) with a single emission that mirrors upstream's `$$IsomorphicComponent` pattern. The existing iso-interface branch (:2029-2096) stays; the 4-arm fallback is deleted.

**Template:**

```ts
/* per-component */
interface $$IsomorphicComponent {
    new <G?>(options: ComponentConstructorOptions<PROPS & { children?: any }>):
        import('svelte').SvelteComponent<PROPS, EVENTS, SLOTS>
          & EXPORTS
          & { $$bindings?: BINDINGS };
    <G?>(internal: unknown, props: PROPS): EXPORTS;
    z_$$bindings?: BINDINGS;
}
const __svn_component_default: $$IsomorphicComponent = null as any;
type __svn_component_default<G?> = InstanceType<typeof __svn_component_default<G?>>;
export default __svn_component_default;
```

**Slot-fillings:**

- `PROPS`:
  - With generics + Props type source + class wrapper present: `ReturnType<__svn_Render_<hash><G>['props']>` (what we already do in the iso branch).
  - No generics, user Props present + safe at module scope: `UserProps & __SvnSvelte4PropsWiden<UserProps>` (Arm 2's props shape lifted).
  - Fallback: `Awaited<ReturnType<typeof $$render_<hash>>>['props']` (Arm 4's shape lifted).
- `EVENTS` / `SLOTS` / `BINDINGS`: same conditional — use the class-wrapper projection when present, or `Awaited<ReturnType<typeof $$render>>['events' | 'slots' | 'bindings']` otherwise. Bindings defaults to `string` when nothing is declared.
- `EXPORTS`: from the existing `build_exports_object` logic — or `{}` when no exports.

**Why it unifies:** the iso-interface pattern doesn't REQUIRE generics or a user Props type source. The `Awaited<ReturnType<typeof $$render>>` projection works for the no-generic case too. The class-wrapper is ONLY needed when generics + Props come from user-scope; for the non-class-wrapper cases, project directly off `$$render`.

**Preserves `typed_events_intersection` marker:** keep emitting `& { readonly __svn_events: $$Events }` when `has_strict_events(doc)`. Upstream uses `Props['$$events']` phantom-field instead; our explicit marker works because the intersection is on the VALUE's type (not the instance's). Both patterns are observationally equivalent at type-check time. Switch mechanisms only if it simplifies the shim; for now keep the marker so `__SvnInstanceTyped` paths continue to work (see §2.3).

**Class-wrapper emission (:1844-1886) stays as-is.** It's a projection helper useful for body-local `typeof X` refs in $$Props; gate moves to "always emit when generics are present", not "only when use_iso_interface fires".

**Output shape for TransformContext** (no generics, no Props type source):

Before (Arm 4, :2186-2192):
```ts
declare const __svn_component_default: import('svelte').Component<Partial<Awaited<ReturnType<typeof $$render>>['props'] & PropsWiden>, Awaited<...>['exports']>;
declare type __svn_component_default = import('svelte').SvelteComponent<Partial<Awaited<...>['props'] & PropsWiden>> & Awaited<...>['exports'];
```

After (unified):
```ts
interface $$IsomorphicComponent {
    new (options: ComponentConstructorOptions<Partial<Awaited<ReturnType<typeof $$render>>['props'] & PropsWiden> & { children?: any }>):
        import('svelte').SvelteComponent<Partial<Awaited<...>['props'] & PropsWiden>, Awaited<...>['events'], Awaited<...>['slots']>
          & Awaited<...>['exports']
          & { $$bindings?: string };
    (internal: unknown, props: Partial<Awaited<...>['props'] & PropsWiden>): Awaited<...>['exports'];
    z_$$bindings?: string;
}
const __svn_component_default: $$IsomorphicComponent = null as any;
type __svn_component_default = InstanceType<typeof __svn_component_default>;
export default __svn_component_default;
```

Consumer `let x: TransformContext` now resolves to `InstanceType<typeof TransformContext>` = the `new` signature's return = `SvelteComponent<...> & Exports & {$$bindings?: string}`. Matches what `new __svn_C(...)({...})` produces (after §2.2).

### 2.2 Change #2 — shim: unified `__svn_ensure_component` + remove `__SvnInstance<P>` as a dedicated shape

**Files:** `crates/typecheck/src/svelte_shims_core.d.ts`.

**What it becomes:** replace the 5 overloads with a SINGLE conditional-return overload, mirroring upstream's structure. The instance shape returned is `SvelteComponent<Props, Events, Slots> & Exports & { $$bindings?: Bindings }` — identical to what the new default-export emission produces.

```ts
declare function __svn_ensure_component<
    T extends
        | (new (...args: any[]) => any)  // Svelte-4 class constructor
        | import('svelte').Component<any, any, any>
        | ((anchor: any, props: any) => any)  // function-form fallback
        | null
        | undefined
>(c: T): NonNullable<
    T extends new (...args: any[]) => any
        ? T
    : T extends import('svelte').Component<infer P extends Record<string, any>, infer E extends Record<string, any>, infer B extends string>
        ? new (options: { target?: any; props?: P }) =>
            import('svelte').SvelteComponent<P, P['$$events'] & {}, P['$$slots'] & {}>
              & E
              & { $$bindings?: B }
    : T extends (anchor: any, props: infer P) => any
        ? new (options: { target?: any; props?: P }) =>
            import('svelte').SvelteComponent<P extends Record<string, any> ? P : {}>
    : never>;
```

**Notes on the shape:**

- Drop `__SvnInstance<P>` / `__SvnInstanceTyped<P, E>` entirely. Every emit-side site that references them (grep for `__SvnInstance` in crates/emit/src/lib.rs) must switch to reading off the concrete instance — mostly `$$prop_def: P` reads, which work identically on `SvelteComponent<P>`.
- `$on` semantics are now the `SvelteComponent.$on`'s overloaded form (from Svelte's real declaration):
  - `<K extends Extract<keyof Events, string>>(type: K, callback: (e: Events[K]) => void): () => void`
  - `(type: string, callback: (e: any) => void): () => void`
- **Typed-events compatibility:** upstream's pattern threads `Props['$$events']` into `SvelteComponent`'s Events slot. For user-declared `interface $$Events { myevent: {id: number} }` we need `E['myevent'] = CustomEvent<{id: number}>`, NOT bare `{id: number}`. Upstream handles this via the emit side — the `__sveltets_2_with_any_event` wrapper in `createRenderFunction` intersects Events with `{[evt: string]: CustomEvent<any>}` for non-strict, and for strict it passes the events object that has already been wrapped. See §2.3.

### 2.3 Change #3 — preserve typed-events narrowing (fixture 61)

**The gotcha:** our existing `__SvnInstanceTyped<P, E>.$on<K>(event: K, handler: (e: CustomEvent<E[K]>) => any)` wraps `E[K]` in `CustomEvent<>`. Users declare `interface $$Events { myevent: {id: number} }` (bare payload shape) and write handlers `(e: CustomEvent<{id: number}>)`. The CustomEvent wrapping bridges them.

If we switch to `SvelteComponent<P, E>.$on<K>(type: K, cb: (e: E[K]) => void)` without wrapping, fixture 61's right handler `(e: CustomEvent<{id: number}>)` no longer matches `E[K] = {id: number}` — breaks.

**Fix:** Change where the wrapping happens. Instead of wrapping in the shim's `$on` signature, wrap in the EMIT side's events synthesis — the events object returned from `$$render` gets `{myevent: CustomEvent<{id: number}>}` rather than `{myevent: {id: number}}`.

Concretely: `$$render`'s return (emit/lib.rs:1611) writes `events: undefined as any as {events_field}` where `events_field` is currently `$$Events` (when strict) or `{}`. Change to transform `$$Events` into `{ [K in keyof $$Events]: CustomEvent<$$Events[K]> }` inline:

```ts
events: undefined as any as ({ [K in keyof $$Events]: CustomEvent<$$Events[K]> }),
```

Then `EVENTS = Awaited<ReturnType<typeof $$render>>['events']` picks up the `CustomEvent<...>`-wrapped shape. SvelteComponent's `$on<K>(cb: (e: Events[K]) => void)` now binds `Events[K] = CustomEvent<{id: number}>` — user's handler matches. Fixture 61 passes.

**Alternative:** pre-wrap in the `$$Events` interface declaration itself, so users write `interface $$Events { myevent: CustomEvent<{id: number}> }`. But that breaks Svelte-4 convention where `$$Events` holds bare payloads. Stick with the transform-in-render-return approach.

**One subtle property:** upstream doesn't transform `$$Events` this way — their Svelte-4 shim layers handle the wrapping via `__sveltets_2_with_any_event` + `createEventDispatcher`'s declared types. Inspect upstream's `createRenderFunction.ts` + `svelte-shims-v4.d.ts` for `with_any_event` handling to verify the equivalence. If upstream's flow already produces `CustomEvent<>`-wrapped events in the render return, our transform might already be unnecessary — this needs empirical verification before implementation.

### 2.4 Change #4 (free) — re-enable the TokenMapEntry

Re-apply the `emit_bind_this_assignment` change from 06003398:

```rust
buf.push_str(inner);
buf.append_with_source(expr, *range);  // LHS via TokenMapEntry
buf.push_str(" = ");
buf.push_str(inst_local);
buf.push_str(";\n");
```

After §2.1 + §2.2 land, tsgo no longer fires shim-shape TS2322 on Chart/Pala/datagrid. The TokenMapEntry then surfaces ONLY legitimate errors — fixture 68's explicit `Component<P>` target, plus any other user code with genuinely wrong `bind:this` targets. Matches upstream exactly.

---

## 3. Fixture-first validation (Rule #8)

Before any Rust change, lock upstream's shape as `design/component_bind_this/fixtures/`:

### 3.1 Fixture: `01-default-export-iso`

Standalone TS fixture with the hand-authored iso-interface pattern for a no-generics component. Matching `ParentBindThis.ts` that declares `let ref: TransformContextLike` and assigns `new C({...})`. Must tsgo-clean.

Deliberate-break companion: assign a wrong-shape value → TS2322 at a known position with a known code. Lock.

### 3.2 Fixture: `02-default-export-iso-generics`

Same but with generics `<T>`. Verifies the class-wrapper projection path stays correct.

### 3.3 Fixture: `03-ensure-component-unified`

Standalone TS with the new single-overload `__svn_ensure_component`, called on:
- A class ctor → returns the class.
- A `Component<P, E, B>` → returns a ctor whose instance is `SvelteComponent<P, E, S> & E & {$$bindings?: B}`.
- A function component `(anchor, props) => any` → returns a ctor whose instance is `SvelteComponent<P>`.

Each with a probe `const _: typeof r = …` assignment to a known-shape target, locking the returned instance type.

### 3.4 Fixture: `04-typed-events-preserved`

Child declares `interface $$Events { myevent: {id: number} }`. Parent writes right + wrong handlers. Verify right handler TS-clean, wrong handler TS2345 at the directive position. Lock the CustomEvent-wrapping semantic however we end up implementing it (§2.3).

### 3.5 Fixture: `05-bind-this-real-world`

Chart.svelte-style: parent declares `let ref: MyComp`, `<MyComp bind:this={ref}>`. Verify clean. Mutation: wrong-type `ref: WrongComp` → TS2322 at `bind:this={ref}` source position with TokenMapEntry active.

All five must gate green BEFORE any Rust change ships.

---

## 4. Staged rollout

Each stage ends in a green `cargo test --test emit_snapshots`, a green `cargo test --test bug_fixtures` (including fixture 61 typed events + fixture 68 bind-this), and a bench check that doesn't regress beyond the staged targets. Commit after each.

**Stage 0 — fixtures.** Write the five tsgo-gated fixtures in `design/component_bind_this/fixtures/`. All five must hand-compile green + deliberate-break fire the right diagnostics. No Rust.

**Stage 1 — default-export emit unification.** Delete the 4-arm match. Emit the iso-interface pattern for every TS-overlay component, regardless of generics / Props source. Shim unchanged. Snapshots move mechanically (~30-50 expected) — every non-iso component gains the interface shape. Benches: expect **no change** in the error counts yet (the shim still returns `__SvnInstance<P>` which still fails the target). Commit.

**Stage 2 — shim: unified `__svn_ensure_component` + drop `__SvnInstance`.** Replace the 5 overloads with one. Delete `__SvnInstance<P>` / `__SvnInstanceTyped<P, E>`. Update every emit-side reference (`$$prop_def: P` reads stay valid via SvelteComponent). At this point the INSTANCE from `new __svn_C({...})` is `SvelteComponent<P, Events, Slots> & Exports & {$$bindings?: B}` — matches Stage-1's target shape. Bench: **layerchart should drop from 31 → 30** (Chart.svelte clears). Palacms 427 → 426, datagrid 64 → 63. Commit.

**Stage 3 — events wrapping audit.** Verify fixture 61 (typed events narrow + wrong-handler TS2345) still passes. If not — implement the `events:` render-return transform in §2.3. Commit.

**Stage 4 — re-enable TokenMapEntry.** Flip the `emit_bind_this_assignment` back to `append_with_source`. Fixture 68 asserts TS2322 at `Parent.svelte 12:30` — matches upstream. Bench: no new surfaces (Stage 2 eliminated the over-fires). Commit.

**Stage 5 — cleanup.** Delete any `__SvnInstance` / `__SvnInstanceTyped` / `__svn_events` marker code paths now unused. Re-run the full bench fleet. Commit.

Between each stage, if any bench regresses by more than 2 errors, STOP. Diff the affected overlay vs upstream on one file, read the shim output, don't guess.

---

## 5. Risks

- **R1: Snapshots move broadly.** Every component's default-export declaration text changes. UPDATE_SNAPSHOTS=1 auto-accepts; reviewers eyeball one representative per cluster (with/without generics, with/without Props type, with/without exports) to verify shape parity with upstream.

- **R2: Removing `__SvnInstance<P>` breaks readers.** Grep finds ~15 references to `__SvnInstance` in emit. Most are incidental (`typeof __SvnInstance<P>['$$prop_def']` extractions for template-walker refs). Audit each; switch to direct `SvelteComponent<P>['$$prop_def']` reads or `Awaited<ReturnType<typeof $$render>>['props']` where possible.

- **R3: The `Props['$$events']` vs `__svn_events` marker split.** Upstream reads Events from `Props['$$events']` (phantom field). Ours uses explicit intersection marker. If we keep our marker in Stage 2, the shim's conditional return must read from the right source. Two-way compat sketch:
  ```ts
  T extends Component<infer P, infer E, infer B> & { readonly __svn_events: infer E2 }
      ? new (…) => SvelteComponent<P, E2, P['$$slots'] & {}> & E & { $$bindings?: B }
  : T extends Component<infer P, infer E, infer B>
      ? new (…) => SvelteComponent<P, P['$$events'] & {}, P['$$slots'] & {}> & E & { $$bindings?: B }
  : …
  ```
  Match the existing marker first, fall through to phantom field. Keeps fixture 61 green.

- **R4: Overload 472 (constructor passthrough).** Svelte-4 pure class components (extending `SvelteComponent`) still need the identity passthrough — the shim's return IS the class itself, `new Class(options)` gives `InstanceType<Class>` without shim transformation. Preserve this in the conditional return's first branch. Any `ConstructorOfATypedSvelteComponent` input passes through unchanged.

- **R5: `Component<P>` callable-only detection.** Svelte 5's real `Component<P>` is declared as a pure function type with no `new` signature. Our shim's overload 486 matches this. After unification, the single conditional return must still identify `T extends Component<…>` and produce a NEW constructor — which requires our returned type to ADD a `new` signature. Upstream does exactly this. Verify with a minimal TS probe that `T extends Component<infer P, …>` inference triggers correctly on Svelte 5's `Component<Props>`.

- **R6: `has_bubbled_component_event` widen.** The `prop_type_effective = Some("Record<string, any>")` override (emit/lib.rs:702-705) still applies in Stage 1. It widens the PROPS slot but doesn't touch events/slots/exports. Verify that wide Props still flows through the iso interface correctly.

---

## 6. Kill criteria

Abandon the port and revert to pre-Stage-1 state if any of:

- **K1**: Stage 1 moves >100 snapshots (>50% of the svelte2tsx corpus). Indicates deeper divergence than a mechanical swap — the 4 arms likely encoded important variation we can't faithfully replace with one shape.
- **K2**: After Stage 2, layerchart DOESN'T drop from 31 → 30. Means the shim unification doesn't produce the expected instance shape — implementation is wrong; don't ship a broken bench.
- **K3**: Fixture 61 (typed events) breaks and Stage 3 can't restore it without >30 LOC of shim conditional logic. Signals the phantom-field vs explicit-marker gap isn't closable cleanly.
- **K4**: Any control-bench gains >3 errors (control-svelte-4 admin-app, control-svelte-5 admin-app). These are parity gates; a regression there means real user code breaks.

If killed, document the specific failure in `notes/DEFERRED.md` under "Component bind:this isomorphic port" with the stage that blocked + the evidence that blocked it.

---

## 7. Files touched (projection)

**New:**
- `design/component_bind_this/fixtures/01-05-*/` — five tsgo-gated fixtures.

**Modified:**
- `crates/emit/src/lib.rs`:
  - `emit_default_export_declarations` (:1747-2212) — major rewrite, delete 4-arm match.
  - `emit_render_body_return` (:1598-1611) — events-wrapping if §2.3's transform needed.
  - `emit_bind_this_assignment` — re-enable TokenMapEntry.
  - Grep-and-replace `__SvnInstance` → direct SvelteComponent reads (Stage 2).
- `crates/typecheck/src/svelte_shims_core.d.ts` (:472-553) — replace overloads + delete `__SvnInstance` / `__SvnInstanceTyped`.

**Snapshots** (UPDATE_SNAPSHOTS=1):
- Every `crates/cli/tests/emit_snapshots/*/*/expected.emit.ts` whose overlay includes a default-export declaration.

**No change:**
- Parser.
- Kit-file injection.
- Upstream sanity test adapter.

---

## 8. What to do if you're implementing this

1. Read `CLAUDE.md` (Rules #1, #3, #8 especially), then this plan.
2. Write Stage 0's fixtures FIRST. Commit before any Rust change. If any fixture can't hand-compile green, the theory is wrong — fix the plan.
3. Stage 1 IS mostly a deletion + unification of existing templates. The iso branch already emits the shape we want — the work is threading the right PROPS/EVENTS/SLOTS/EXPORTS/BINDINGS sources through for the non-iso branches. Grep for every format-string slot-fill and make sure it picks up the Awaited projection fallback when generics/class-wrapper aren't present.
4. Stage 2: re-verify overload resolution. If TS doesn't cleanly pick up the conditional branches, add a `null | undefined` exclusion and check for Svelte-5 mode detection (`typeof import('svelte') extends { mount: any }` is upstream's pattern).
5. Run `bench.mjs --target /path/to/layerchart` between stages to catch regressions early. The "parity-gate" is: layerchart errors monotonically non-increasing after Stage 1.
6. If any stage blocks more than half a day, STOP and escalate via OPEN.md. The kill criteria are the objective fallback — apply them rather than debugging indefinitely.

If implementation ships successfully, remove the dedicated OPEN.md entry for this work and note in HISTORY.md as closed.
