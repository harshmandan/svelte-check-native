# Session context for next Claude Code session

This document summarizes what was done across several sessions so a fresh Claude Code run has the full picture. Read this first, then open `todo.md` for the forward plan.

---

## TL;DR

Goal: we're building **`svelte-check-native`** — a clean-room rewrite of a Rust-based svelte-check replacement. The directory `~/Documents/GitHub/svelte-check-native/` is already set up with:

- `upstream/` — a patched fork we fixed 33+ bugs in. **Reference only.** Do not copy code from it verbatim.
- `language-tools/` — clone of `sveltejs/language-tools` (contains `packages/svelte-check` as upstream source of truth).
- `todo.md` — the detailed phase-by-phase plan to follow.

No `git init` has happened yet.

---

## Where this came from

### 1.  v4 needed faster type checking

Working in `~/Documents/GitHub/` (SvelteKit + TypeScript monorepo). Regular `tsc -b` took ~27s, `svelte-check` took ~48s — painful for AI agents running checks in a loop.

### 2. We set up fast type checking (landed on `main` of )

Commit `c2ca500c1b` on 's `main` added:
- **`tsgo` (`@typescript/native-preview`)** — Go-based TypeScript compiler preview. Added at workspace root, usable via `bun typecheck:fast` (maps to `tsgo -b`). ~4x faster than `tsc`.
- **`upstream`** — Rust-based svelte-check replacement using tsgo under the hood. Added to `the benchmark project` only, usable via `bun the benchmark project:check:fast`.
- **Turbo `check:fast` task** — mirrors `check` but calls the fast variant per package.
- **Claude Code hook** at `.claude/hooks/enforce-package-scripts.sh` — rewrites AI-agent bash commands so `bun typecheck` → `bun typecheck:fast` and `bun turbo check` / `bun check` / svelte-check invocations → `bun turbo check:fast`. Handles compound commands (`&&`, pipes) via in-place substitution.
- **CI updates** — `workspace-check.yml` and `the benchmark project.yml` now use `:fast` variants.
- **Pre-push hook** updated to use `:fast` variants.

Benchmarks (the benchmark project):
- `tsc -b`: ~27s → `tsgo -b`: ~7s (4.3× faster warm)
- `svelte-check`: ~48s → `upstream` with our fixes: ~7s (7× faster)

### 3. upstream had many bugs on real Svelte 5 code

We cloned `external/upstream` locally, diagnosed 33+ bugs, fixed them in the Rust source, rebuilt, and ran against the benchmark project. Went from **820 errors → 0 errors** on our large codebase.

### 4. We couldn't upstream the fork

The user (harsh) doesn't want to maintain a public fork of someone else's repo. Decision: write our own from scratch, **but** informed by what we learned and keeping the patched `upstream` in-tree for reference.

### 5. Repo structure as it stands now

```
~/Documents/GitHub/svelte-check-native/
├── upstream/                  # patched fork (reference, will be gitignored)
│   ├── crates/
│   │   ├── source-map/
│   │   ├── svelte-parser/
│   │   ├── svelte-transformer/       # where most bugs lived
│   │   ├── svelte-diagnostics/
│   │   ├── tsgo-runner/
│   │   ├── bun-runner/
│   │   └── upstream/          # CLI
│   ├── report.md                     # detailed list of all 33 fixes
│   └── target/release/upstream # patched binary ( uses this)
├── language-tools/                   # cloned from github.com/sveltejs/language-tools
│   └── packages/svelte-check/        # upstream we want parity with
│       ├── src/                      # TypeScript source
│       ├── test-success/             # passing fixtures
│       └── test-error/               # failing fixtures + expected output
├── todo.md                           # the plan
└── SESSION_CONTEXT.md                # this file
```

Neither `git init` nor `cargo init` has been run yet. The clone of language-tools has its own `.git` — we'll convert that to a submodule when we initialize the outer repo.

---

## The 33+ bugs we fixed in `upstream/`

Each of these is a test case we need to reproduce in the new codebase. Full details are in `upstream/report.md`.

### Transformer / store detection
1. `$` in identifiers: `parent$` store got truncated → generated code declares `$parent` but source uses `$parent$`
2. Rune classification: `$foo(...)` incorrectly classified as rune regardless of name → callable stores like svelte-i18n's `$t(...)` broke
3. Transition/animate directive names never referenced → `transition:fade` flagged `fade` import as unused

### Template type-checking
4. `__svelte_template_check__` declared but never called → TS6133
5. Action attrs `__action_attrs_N` not referenced → TS6133
6. `__bind_pair_N` declared but never used → TS6133
7. Style directive shorthand `style:height` never referenced `height`
8. Style interpolations in quoted strings: `style:left="{x}px"` treated value as plain string, never extracted `{x}`

### Props
9. Parser couldn't handle comments between destructured props
10. Renames parsed as type annotations (`class: classValue` treated wrong)
11. No `local_name` on renamed props → void references used wrong identifier
12. Generic args in props broke parser: `$bindable<Record<string, UserRole>>({})` got split on inner commas
13. Fallback `$props()` type too strict (`Record<string, unknown>`) → changed to `Record<string, any>`
14. Destructured props not referenced → only-used-in-bind props flagged unused

### svelte:* elements
15. `<svelte:element>` attributes silently skipped → props like `{href}` flagged unused
16. `<svelte:document|window|body>` event handlers not referenced

### Component types
17. Svelte component default export not a type → `import type X from './Comp.svelte'` failed TS2749
18. Generic components lost their generics in type alias → `VirtualList<T>` broke
19. `__SvelteComponent` didn't overlap with Svelte's `Component` → `as Component` casts failed TS2352

### SvelteKit
20. `PageProps`/`LayoutProps` imported but unused when user provides own Props type
21. `RequestHandler` param type injection conflicted with user's explicit `: RequestHandler` on the const

### HTML elements
22. Unknown tag names errored on `ElementTagNameMap["foo"]`
23. `bind:this` on custom elements typed as HTMLElement but users often annotate HTMLDivElement
24. Actions on `<svelte:window>` failed because `Action<Element>` doesn't accept `Window`

### `{#each}` / stores
25. Empty `{#each items}` pattern → `for (const  of ...)` syntax error
26. ArrayLike in each: `{#each { length: N } as item}` failed Symbol.iterator
27. `__SvelteEachItem<any>` resolved to `unknown` due to conditional-type distribution over `any`
28. Auto-subscribed stores flagged unused when only written (`$store = value` sugar)

### Component bindings
29. `bind:` on component never referenced the bound variable → TS6133
30. Bound variables flagged "used before assigned" in script closures — fixed with `let x!: T` definite-assignment rewriter

### Snippets / actions / modules
31. Default `children:` snippet duplicated when user had `{#snippet children()}` → TS1117
32. `__svelte_ensure_action` return type used `T["$$_attributes"]` instead of `NonNullable<T["$$_attributes"]>` — since `$$_attributes?` is optional, `undefined` leaked into `__svelte_union` intersection → `never`
33. Excluding source `.svelte.ts` files from the tsconfig overlay caused TS6307 when imports like `./foo.svelte` resolved to source paths

Plus ~10 -side fixes we pushed to ``'s `main`:
- `disablePictureInPicture` → `disablepictureinpicture` (×3 video players — Svelte types are lowercase)
- `data-sveltekit-preload-code="false"` → `"off"`
- +server.ts imports using generic `RequestEvent` instead of route-specific `RequestHandler`
- Unused destructures, implicit `any` callbacks, narrowing bugs in `false | number` / `false | string` unions
- `$state(undefined)` → `$state<T | undefined>(undefined)` for better inference
- Complex nested template literal in `<svelte:head>` → `$derived.by` callback

Those  fixes landed as two commits on `main`: `e0116f8f39` and `65e998d8bf`.

---

## Current state of related repos

### `~/Documents/GitHub/`
- Branch: `harsh/fast-typecheck-setup` (has typecheck:fast setup)
- Latest commits: `01b86e16c7` (cherry-picked type inference fixes), `afe31d9fe0` (cherry-picked real TS fixes), `c2ca500c1b` (fast typecheck setup)
- PR #6090 open from this branch
- `main` has the two  fix commits (`e0116f8f39`, `65e998d8bf`)

### `~/Documents/GitHub/svelte-check-native`
- No git yet
- Contains `upstream/`, `language-tools/`, `todo.md`, `SESSION_CONTEXT.md`

---

## What the next session should do

Work through `todo.md`. Phases in order:

1. **Phase 0** — study upstream (`language-tools/packages/svelte-check/src`), then `git init` + cargo workspace + submodule conversion.
2. **Phase 1** — build the 8 crates (`core`, `parser`, `analyze`, `emit`, `lint`, `svelte-compiler`, `typecheck`, `cli`) with fresh naming and single-pass analysis.
3. **Phase 2** — one test fixture per bug from the list above. Tests are spec-first: write the expected behavior before the impl.
4. **Phase 3** — CLI parity (auto-detect tsconfig, `--diagnostic-sources`, `key:severity` format for `--compiler-warnings`, etc.).
5. **Phase 4** — consume `language-tools/packages/svelte-check/test-{success,error}/` fixtures directly as our parity suite.
6. **Phase 5** — performance (fused passes, interned symbols, rayon parallelism, caching).
7. **Phase 6** — npm wrapper + CI matrix releases.
8. **Phase 7** — docs.

---

## Key constraints (don't forget)

- **`upstream/` is reference only.** Reading + diffing is fine. Copy-pasting code, mirroring file names, or keeping identical type signatures is not. The rename map in `todo.md` shows the convention — internal helpers go from `__svelte_*` to `__svn_*`, `Span` → `Range`, `TemplateContext` → `EmitContext`, etc.
- **Upstream svelte-check CLI is the source of truth for flags and output format.** Diverging requires an explicit note in `docs/svelte-check-compat.md`.
- **tsgo is a nightly preview.** Some issues we fixed (like `any` distribution) may get fixed upstream — keep our workarounds clearly commented.
- **Our testing bar is higher than the original.** They had 88 snapshot tests that locked in buggy behavior. We write expectation-first tests.

---

## Useful commands for orientation

```bash
# See what upstream svelte-check looks like
cd ~/Documents/GitHub/svelte-check-native/language-tools/packages/svelte-check
ls src/  # entry is src/index.ts

# See the patched fork for reference
cd ~/Documents/GitHub/svelte-check-native/upstream
cat report.md  # full bug fix writeup
cat crates/svelte-transformer/src/template.rs | head -60

# Verify the patched binary still works against 
cd ~/Documents/GitHub//src/apps/the benchmark project
~/Documents/GitHub/svelte-check-native/upstream/target/release/upstream \
  --tsconfig tsconfig.json --threshold error
# Expected: "upstream found 0 errors and 0 warnings in 0 files"
```
