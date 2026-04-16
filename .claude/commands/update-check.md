Check whether upstream `language-tools` has changed in ways that affect
`svelte-check-native`'s correctness, CLI surface, or output formats.

## Steps (do not skip or reorder)

1. Read the pinned SHA from `.upstream-pin` — the `sha = "..."` line
   under `[language-tools]`. Call this `<PINNED>`.

2. Fetch upstream without merging:
   ```
   git -C language-tools fetch origin
   ```
   Resolve `origin/HEAD`'s SHA — call this `<NEW>`. If `<NEW>` ==
   `<PINNED>`, report "no upstream changes since last review" and stop.

3. List commits landed since the pin:
   ```
   git -C language-tools log --oneline <PINNED>..origin/HEAD
   ```

4. Diff the surface that matters. Run each command, capture summary:
   ```
   # CLI surface (flags, options, output formats):
   git -C language-tools diff --stat <PINNED>..origin/HEAD \
     -- packages/svelte-check/src/

   # svelte2tsx behavior (informs our emit crate):
   git -C language-tools diff --stat <PINNED>..origin/HEAD \
     -- packages/svelte2tsx/src/

   # tsconfig handling shared by both:
   git -C language-tools diff --stat <PINNED>..origin/HEAD \
     -- packages/svelte-check/src/svelte-check.ts \
        packages/svelte-check/src/options.ts
   ```

5. Classify each landed commit into ONE of:
   - **CLI / output surface** (svelte-check/src/options.ts,
     interfaces.ts, output formatters) — mirror to our cli + typecheck
     output crates.
   - **svelte2tsx emit shape** (svelte2tsx/src/) — may require parity
     work in `crates/emit/`.
   - **svelte/compiler integration** — affects the bridge in
     `crates/svelte-compiler/`.
   - **New `.v5` test fixtures** — handled by `/update-tests`, just
     note the count here.
   - **Infrastructure / docs / CI** — ignore for our purposes.

6. Summarize:
   - One line per category with the commits in it (use `<short-sha>
     <subject>` form).
   - For each non-ignorable category, list the specific files in our
     repo that may need updating, with a one-line "why".

7. Decide and report:
   - **No work needed** → tell the user it's safe to bump the pin and
     the submodule gitlink to `<NEW>`. Provide the exact commands:
     ```
     git -C language-tools checkout <NEW>
     git add language-tools
     # Then update .upstream-pin's [language-tools] sha = "<NEW>"
     ```
   - **Work needed** → list the work as a checklist. Tell the user to
     re-run `/update-check` after the work is done.

## Hard rules

- NEVER bump `.upstream-pin` automatically. The user has to confirm
  after reading your report.
- NEVER change the submodule gitlink (`git submodule update --remote`,
  `git -C language-tools checkout <new>`) automatically.
- NEVER modify upstream files inside `language-tools/` — it's a
  read-only reference checkout.
- If the diff is large (50+ commits), summarize aggressively rather
  than dumping every commit. The user wants a verdict, not a log.
