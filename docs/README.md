# docs/

Public-facing design notes and investigation logs. Anything here is
safe to link to from a PR, an issue, a release note, or external
docs — unlike `notes/` (gitignored scratch) this directory ships with
the repo.

## Contents

- [`parity-findings-2026-04-21.md`](./parity-findings-2026-04-21.md)
  — investigation trail for three `v0.3.9` emit fixes driven by a
  SvelteKit sub-app user report. Documents the `use:enhance`
  callback-typing miss, the `{#if form?.success}{form.error}`
  narrowing miss, the paraglide literal-key miss, and where
  SvelteKit's `ActionData` deliberately widens cross-branch property
  access (a class of "miss" that's actually upstream behavior).

## Where else to look

- [`../README.md`](../README.md) — user-facing project overview and
  install instructions.
- [`../CHANGELOG.md`](../CHANGELOG.md) — per-version release notes.
- [`../CLAUDE.md`](../CLAUDE.md) — internal engineering conventions
  and architecture rules. Shipped for transparency but written for
  contributors, not end users.
- [`../design/`](../design/) — tsgo-validated emit-shape fixtures
  (per CLAUDE.md rule #8). Each subdirectory contains a hand-written
  TS fixture that proves a specific emit shape produces the intended
  diagnostics before we translate it to Rust.
