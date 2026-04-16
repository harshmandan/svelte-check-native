# svelte-check-native — plan

What's still ahead. For project goals, conventions, and architecture
see `README.md` (public-facing) and `CLAUDE.md` (engineering context).

## Current scoreboard

```
v5 fixtures (svelte2tsx upstream corpus):  59 / 63 passing  (51 clean + 8 within-baseline)
v5 stores fixtures (local port):           24 / 24 passing  (14 clean + 10 within-baseline)
grey-box bug fixtures:                      6 / 6  passing
unit tests:                                 cargo test --workspace clean (except upstream_sanity)
fmt + clippy:                               cargo fmt --check + cargo clippy --workspace --all-targets -- -D warnings clean

real-world parity (a representative SvelteKit project, ~1200 .svelte files):
  with default sources:                    0 errors, 44 warnings, ~3 s warm   ← OURS
  matching upstream filter flags:          0 errors, 10 warnings, 7 files     ← byte-equivalent to upstream svelte-check
```

## v0.1 gates — all met

1. ✅ **Zero errors** on the benchmark project — matches upstream.
2. ✅ **Real source maps** — per-region line maps; no offset hack.
3. ✅ **Byte-compatible output formats** — `human`, `human-verbose`,
   `machine`, `machine-verbose` all match upstream.
4. ✅ **Compiler warnings emitted** via the multi-worker `svelte/compiler`
   bridge.

## Next up — post-v0.1

### Enrich baselines from `max_errors` to exact-shape assertions

Today a baseline says `"max_errors": 3` and any 3 errors pass. That
catches the count regressing but accepts an emit change that swaps one
expected error for a different one — we'd miss the regression.

Goal: change every baseline entry from `max_errors` to a list of
expected diagnostics, each `{ code, line, column, message contains "..." }`.
Runner asserts the exact set is present (no more, no fewer).

Scope: ~half-day, contained to:
- `crates/cli/tests/v5_fixtures/baselines.json` (8 entries)
- `crates/cli/tests/v5_stores_fixtures/baselines.json` (10 entries)
- `crates/cli/tests/v5_fixtures/run.cjs` (runner)

For each baselined fixture, populate the expected-diagnostics list by
running the binary against it once and capturing machine-verbose output.

NOTE: byte-identical match with `svelte2tsx`'s `expectedv2.ts`
snapshots is a SEPARATE, much larger goal (v0.3+). Not in scope here.

### Fix the pre-existing upstream_sanity test

`crates/cli/tests/upstream_sanity.rs` has 3 failing subcases ("project
with errors" cold/warm/dirty) — pre-existing, unrelated to recent perf
work. Investigate whether the expected error fixtures still apply.

### Tier-2 perf follow-ups (low priority, low impact)

- `format!` + `push_str` sweep in `crates/emit/` — verify nothing in
  the hot path uses the antipattern. Quick audit, no perf goal.
- Profile after multi-worker bridge under varied workloads to confirm
  `cores/2` default holds up on 16-core+ boxes.

## Phase 6 — Packaging (local only, no remote pushes)

- npm wrapper at `npm/svelte-check-native/` following the
  `@typescript/native-preview` pattern: platform-specific packages
  (`-darwin-arm64`, `-darwin-x64`, `-linux-x64`, `-linux-arm64`,
  `-win32-x64`) as optionalDependencies; tiny JS wrapper in main
  package that spawns the binary.
- `cargo-dist` for cross-builds.
- `.github/workflows/{ci,release,parity}.yml` committed but dormant
  until repo exists on GitHub.

## Phase 7 — Docs

- `README.md` — done (slimmed in recent pass).
- `CLAUDE.md` — done (technical reference absorbed).
- `docs/svelte-check-compat.md` — compatibility matrix (which upstream
  flags we support, deviations).

## Phase 8 — Release & publish (the last step)

**Gate to start:** v5 fixtures at 63/63 (or close, with documented
exceptions), benchmark project reports `0 errors, 0 warnings`,
fmt + clippy clean.

Every prior phase is local-only. This is where we first touch a remote.

1. Bump version `0.0.0` → `0.1.0` across workspace.
2. Write `CHANGELOG.md`.
3. `gh repo create harshmandan/svelte-check-native --public`.
4. `git remote add origin git@github.com:harshmandan/svelte-check-native.git`.
5. `git push -u origin main`.
6. Verify CI green.
7. Tag `v0.1.0`, push tag.
8. `cargo-dist` builds platform binaries and creates GitHub Release.
9. Publish platform packages to npm, then main package referencing
   them as optionalDependencies.
10. Optionally `cargo publish` each crate in dependency order.
