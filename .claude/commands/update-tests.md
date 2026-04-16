Find new upstream tests we should mirror, and detect drift in tests
we've already ported.

We track two upstream test corpora:

- **`.v5` fixtures** at
  `language-tools/packages/svelte2tsx/test/svelte2tsx/samples/*.v5/` —
  consumed *directly* by `crates/cli/tests/v5_fixtures.rs`. New ones
  are picked up automatically; we just need to verify they pass and
  baseline if not.

- **Svelte-4 store fixtures** at
  `language-tools/packages/svelte2tsx/test/svelte2tsx/samples/`
  matching `$store-*`, `store-*`, `stores-*`, `*-$store*`, `binding-*-store`,
  `await-with-$store`, `typeof-$store`, etc. — we maintain hand-converted
  Svelte 5 ports under `fixtures/v5-stores/`. New upstream entries
  here need a manual port; modified ones may need our port refreshed.

## Steps (do not skip or reorder)

1. Read `<PINNED>` from `.upstream-pin` (`[language-tools]` sha).

2. Fetch upstream:
   ```
   git -C language-tools fetch origin
   ```
   Resolve `<NEW>` = `git -C language-tools rev-parse origin/HEAD`. If
   `<NEW>` == `<PINNED>`, report "no upstream test changes" and stop.

3. List samples-dir changes:
   ```
   git -C language-tools diff --name-status <PINNED>..origin/HEAD \
     -- packages/svelte2tsx/test/svelte2tsx/samples/
   ```

4. Bucket each path. The first directory component under `samples/` is
   the fixture name. Bucket by:
   - **Status `A`** (added):
     - Ends in `.v5/` → **new v5 fixture, auto-picked-up by our
       runner.** Just note for the user: run `cargo test -p
       svelte-check-native --test v5_fixtures` and check whether it
       passes; if not, add a baseline entry with reason.
     - Matches a store-pattern name (`$store-*`, `store-*`, `stores-*`,
       `await-with-$store`, `binding-assignment-$store`,
       `binding-group-store`, `custom-css-properties-with-$store`,
       `typeof-$store`, `uses-$store*`) → **needs a manual Svelte 5
       port**; flag for the user.
     - Other → note it but don't recommend action (likely irrelevant
       to our scope).
   - **Status `M`** (modified):
     - If we have a port at `fixtures/v5-stores/<name>/` → suggest
       refreshing the port. Show a `git diff <PINNED>..origin/HEAD --
       packages/svelte2tsx/test/svelte2tsx/samples/<name>/` of the
       upstream change so the user can judge whether to mirror it.
     - If `.v5/` and not in our local list → still consumed by our
       runner; just note "may need re-baseline".
   - **Status `D`** (deleted):
     - If we have a corresponding local port → recommend deleting our
       copy and the matching baseline entry.

5. Cross-check our local v5-stores ports against upstream's current
   state for completeness:
   ```
   ls fixtures/v5-stores/         # our ports
   git -C language-tools ls-tree --name-only origin/HEAD \
     packages/svelte2tsx/test/svelte2tsx/samples/ | grep -E '^(\$store|store|stores|await-with-\$store|binding-.*store|typeof-\$store|uses-\$store|custom-css-properties-with-\$store)'
   ```
   Set-diff: anything upstream has but we don't is a **port gap**.

6. Summarize as a checklist for the user:
   - **New v5 fixtures (auto):** count, names — just run the test.
   - **New ports needed (manual):** names, reason, suggested baseline
     entry shape.
   - **Existing ports to refresh:** names with one-line description of
     upstream change.
   - **Ports to delete:** names that upstream removed.

## Hard rules

- NEVER edit `fixtures/v5-stores/` content automatically. Manual ports
  require human translation of Svelte 4 syntax (`on:click`, `export
  let`, `$:`, `<slot>`) to Svelte 5 idiom; a mechanical script would
  produce subtly wrong fixtures.
- NEVER edit `crates/cli/tests/v5_*_fixtures/baselines.json`
  automatically. Baseline entries need a `reason` written by a human
  who understands why the fixture is there.
- NEVER bump `.upstream-pin` or the submodule gitlink. That's
  `/update-check`'s job, and only after both commands report clean.
- If your report would exceed ~50 items, group aggressively
  (e.g. "12 new `.v5` fixtures: <names>").
