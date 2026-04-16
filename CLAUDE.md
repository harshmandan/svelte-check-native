# Working conventions for Claude Code / AI-assisted contributions

This file is loaded into every Claude Code session in this repo. Read `todo.md`
and `SESSION_CONTEXT.md` first — they contain the full project context. This
file is the shorter "rules of engagement" layer on top of that.

## Project at a glance

- **Goal:** a CLI-only type checker for Svelte 5, written in Rust, powered by
  tsgo. Drop-in replacement for upstream `svelte-check`.
- **Reference fork:** `upstream/` is a patched fork of `external/upstream`
  we rescued from. Kept in-tree (gitignored) for diffing. **Never copy code
  from it verbatim.**
- **Upstream submodule:** `language-tools/` is a pinned submodule of
  `sveltejs/language-tools` — the source of truth for CLI behavior.
- **Full plan:** `todo.md`.

## Scope discipline (repeated here because it's easy to forget)

Out of scope — do NOT implement:

- LSP server / editor integration
- Autocomplete, hover, go-to-definition, rename, code actions
- Watch mode (use `watchexec` externally)
- tsc fallback (tsgo only)
- Formatting

In scope: CLI flags matching upstream, byte-identical output formats, tsgo
invocation, diagnostics mapping back to `.svelte` source.

## Commit-and-continue

- **Commit after every meaningful local step,** even if code is broken or tests
  are red. Commits are restore points, not polished artifacts.
- **Never `git push` without explicit user confirmation** each time. Session-level
  approval does not carry over to future sessions or branches.
- Prefer small, frequent commits over large "clean" ones. A half-working
  snapshot is always more valuable than no snapshot.
- Commit message style: imperative mood, lowercase, one concise line. Body
  optional; include when the "why" isn't obvious from the diff.

## Style & quality bar

- **Rust edition 2024.** `rust-version = "1.85"` in every crate's Cargo.toml
  (inherited from workspace).
- `cargo fmt` clean. `cargo clippy -- -D warnings` clean. `cargo test` — the
  scoreboard count must be monotonically non-decreasing per commit.
- No `unwrap()` / `expect()` in library code except with a clear invariant
  comment. Binary entry points (`main.rs`) may use `anyhow::Result` and
  propagate.
- No `TODO:` / `FIXME:` comments checked in without a tracking task in
  `todo.md`. Scratch TODOs belong in a working branch, not main.

## Architecture rules (derived from `-rs` post-mortem — see todo.md)

1. **No character-level scanners for embedded JS/TS.** Use `oxc_parser` and
   walk the AST. `-rs` had ~700 lines of hand-rolled destructuring parser;
   every bug there was an edge case.
2. **Two-phase transformer.** Phase 1 (analyze) populates a `SemanticModel`
   including a `VoidRefRegistry`. Phase 2 (emit) reads from the model. Never
   mutate the model during emit. Never register new names during emit.
3. **Single source of truth for helper types.** `crates/emit/src/helpers.d.ts`
   loaded via `include_str!`. Emitted once to cache; referenced from every
   generated file. Never inlined.
4. **One canonical `TsConfig` struct.** In `crates/core/`. Used by both CLI
   config resolution and overlay builder. No parallel JSON-reading shortcuts.
5. **Pre-allocated buffers.** Estimate output size from AST, allocate
   `Vec<u8>::with_capacity(n)` once. Use `write!` macro, not `format!` +
   `push_str`.
6. **Naming:** our helpers are prefixed `__svn_`. Upstream uses `__sveltets_2_`.
   This is a user-transparent rename and signals non-copying.

## Testing discipline

- **Spec-first.** Write the test before the implementation. Tests live in
  `crates/<crate>/tests/` and `fixtures/` and reference upstream fixtures in
  `language-tools/packages/svelte-check/test-{success,error}/`.
- **Bug fixtures** in `fixtures/bugs/<NN>-<slug>/` — one per the 33 bugs from
  the `-rs` rescue. Each has `input.svelte` + optional `expected.json`.
- **`cargo test` is the scoreboard.** The scoreboard count shown in `README.md`
  is the count of passing integration tests under `crates/cli/tests/`.

## When in doubt

- Read `todo.md` first.
- Check `upstream/` for how the problem *was* solved (but do not copy).
- Check `language-tools/packages/svelte-check/src/` for how upstream solves it.
- Prefer the upstream approach over the fork's.
