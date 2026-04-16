# Working conventions for Claude Code / AI-assisted contributions

This file is loaded into every Claude Code session in this repo. Read
`README.md` and `todo.md` for the full project context. This file is the
shorter "rules of engagement" layer on top of those.

## Project at a glance

- **Goal:** a CLI-only type checker for **Svelte 5+ only**, written in
  Rust, powered by tsgo. Drop-in replacement for upstream `svelte-check`
  on the CLI surface (same flags, same output formats, same exit codes).
- **No Svelte 4 support** — this is a deliberate scope decision. Drops
  `export let foo` prop syntax, `$:` reactive statements, `<slot>`, and
  `on:` event directives from our handling.
- **No bundled tsgo.** We discover the user's `@typescript/native-preview`
  install in `node_modules`, preferring the platform-native binary over
  the JS wrapper. `TSGO_BIN` env var is the override.
- **Upstream submodule:** `language-tools/` is a pinned submodule of
  `sveltejs/language-tools` — used as the source of truth for upstream's
  CLI behavior and for the 63 `.v5` test fixtures from the `svelte2tsx`
  package that form our parity gate.

## Scope discipline (repeated here because it's easy to forget)

Out of scope — do NOT implement:

- Svelte 4 syntax (export let, $:, <slot>, on:event directives)
- LSP server / editor integration
- Autocomplete, hover, go-to-definition, rename, code actions
- Watch mode (use `watchexec` externally)
- tsc fallback (tsgo only)
- Formatting

In scope: CLI flags matching upstream, byte-identical output formats,
tsgo invocation, diagnostics mapping back to `.svelte` source.

## Commit-and-continue

- **Commit after every meaningful local step,** even if code is broken or
  tests are red. Commits are restore points, not polished artifacts.
- **Never `git push` without explicit user confirmation** each time.
  Session-level approval does not carry over to future sessions or
  branches.
- Prefer small, frequent commits over large "clean" ones. A half-working
  snapshot is always more valuable than no snapshot.
- Commit message style: imperative mood, lowercase, one concise line.
  Body optional; include when the "why" isn't obvious from the diff.

## Style & quality bar

- **Rust edition 2024.** `rust-version = "1.85"` in every crate's
  Cargo.toml (inherited from workspace).
- `cargo fmt` clean. `cargo clippy --workspace --all-targets -- -D warnings`
  clean. `cargo test` — the scoreboard count must be monotonically
  non-decreasing per commit.
- No `unwrap()` / `expect()` in library code except with a clear
  invariant comment. Binary entry points (`main.rs`) may use
  `anyhow::Result` and propagate. Test code may use both freely (it's
  supposed to panic loudly on unexpected states).
- No `TODO:` / `FIXME:` comments checked in without a tracking task in
  `todo.md`. Scratch TODOs belong in a working branch, not main.

## Architecture rules

1. **No character-level scanners for embedded JS/TS.** Use `oxc_parser`
   and walk the AST. Hand-rolled destructuring/expression scanners are
   fragile by construction; an AST-level pattern match makes whole
   classes of bug categorically impossible.
2. **Two-phase transformer.** Phase 1 (analyze) populates a
   `SemanticModel` including a `VoidRefRegistry`. Phase 2 (emit) reads
   from the model. Never mutate the model during emit. Never register
   new names during emit.
3. **Single source of truth for synthesized-name registry.** Every name
   the emit crate creates (template-check wrapper, action attrs, bind
   pairs, store aliases, prop locals) is registered once and emitted
   in one consolidated `void (...)` block. No per-feature `void <name>;`
   sprinkling.
4. **One canonical `TsConfig` struct.** In `crates/core/`. Used by both
   CLI config resolution and the overlay builder. No parallel
   JSON-reading shortcuts.
5. **Pre-allocated buffers.** Estimate output size from AST, allocate
   `Vec<u8>::with_capacity(n)` once. Use `write!` macro, not `format!`
   + `push_str`.
6. **Synthesized-name prefix:** `__svn_*`. Used for every name the emit
   crate creates so they're trivially distinguishable from user code in
   diagnostics.

## Testing discipline

- **Spec-first.** Write the test before the implementation. Tests live
  under `crates/<crate>/tests/` and `fixtures/`.
- **Parity corpus:** the 63 `.v5` fixtures under
  `language-tools/packages/svelte2tsx/test/svelte2tsx/samples/*.v5/`.
  Each is a known-good Svelte 5 component; our binary should produce
  zero tsgo errors against any of them.
- **Grey-box regression fixtures** in `fixtures/bugs/<NN>-<slug>/` —
  small focused fixtures targeting specific emit-shape behaviors
  (void-references, definite-assignment rewrites, for-of fallback for
  empty `{#each}`).
- **`cargo test` is the scoreboard.** Count of passing integration
  tests under `crates/cli/tests/` shows in `README.md`.

## When in doubt

- Read `README.md` first for the project overview.
- Read `todo.md` for the implementation plan and architectural decisions.
- Check `language-tools/packages/svelte-check/src/` for how upstream
  solves CLI/output problems.
- Check `language-tools/packages/svelte2tsx/src/` for how the upstream
  Svelte → TS transpilation works (informs our `emit` crate).
