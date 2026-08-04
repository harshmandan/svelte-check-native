# Contributing

Thanks for helping out. This file is the short version; `README.md` has
the user-facing overview and `CLAUDE.md` has the full engineering
conventions.

## Scope

The goal is **parity with upstream `svelte-check`** — same errors, same
warnings, same file counts, same exit codes. Not more correct, not
stricter, not laxer. A change that makes us "more right" than upstream
is a bug report against upstream, not a patch here.

Out of scope: LSP / editor features, watch mode, formatting, tsc
fallback, CSS lint rules.

## Setup

```sh
git clone --recurse-submodules https://github.com/harshmandan/svelte-check-native
cargo build
```

The `language-tools/` submodule is required — it holds the upstream
fixtures the snapshot suite reads. The tsgo-backed suites additionally
need a `node_modules` with `typescript@7` or `@typescript/native-preview`
installed; without one they'll skip or fail locally.

## Gates before opening a PR

```sh
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib          # unit tests
cargo test --test emit_snapshots      # primary emit-shape gate
cargo test                            # everything, if your env has an engine
```

CI runs the first three plus `cargo-deny`; the tsgo-backed suites are
local-only for now, so run them if you can and say in the PR if you
couldn't.

Formatting: run `rustfmt --edition 2024 <file>` on **only the files you
touched**. Bare `cargo fmt` reflows ~17 untouched files on newer
rustfmt versions — `git status` must show only your intended files.

## Every behavior change ships a fixture

No fix lands without a regression lock. Add a directory under
`fixtures/bugs/<NN>-<slug>/` with:

- `input.svelte` (plus any `tsconfig.json` / support files it needs)
- `expected.json` — exact `errors` / `warnings` list, and a `_note`
  explaining what the fixture pins and why

Then regenerate the emit snapshot:

```sh
UPDATE_SNAPSHOTS=1 cargo test --test emit_snapshots
git diff   # review every line before committing
```

Never hand-edit a file under `crates/cli/tests/emit_snapshots/` — it is
generated output. Review the regenerated diff; an unexplained change in
an unrelated snapshot means your change had a wider blast radius than
you thought.

## Test output must be deterministic

Snapshots have to be byte-identical from any checkout directory, on any
OS. Two separate PRs have been merged and then patched for violating
this — a hash keyed on an absolute path, and a file list in filesystem
traversal order. If your change writes anything derived from a path, a
directory listing, a timestamp, or a hash map iteration order into
output, normalize it (workspace-relative paths, `/` separators, explicit
sort) before it reaches a snapshot.

## Diff the real artifact, don't theorize

When our diagnostics diverge from upstream, the first move is to look at
what upstream actually emits — not to reason about what TypeScript
"should" do:

```sh
node scripts/diff-emit.mjs path/to/File.svelte     # upstream overlay vs ours
node scripts/diff-parse.mjs path/to/File.svelte    # parse-tree divergence
```

Then read the `language-tools/` submodule source that produces that
shape. A PR description that names the upstream file it mirrors gets
reviewed a lot faster than one that argues from first principles.

New emit shapes (a new helper, component-call form, binding pattern) are
validated as hand-written TS compiled with tsgo *before* the Rust change
— clean case zero diagnostics, deliberately-broken companion producing
exactly the expected codes at the expected positions.

## What not to touch

- `CHANGELOG.md` and version numbers — maintainer-owned, updated at
  release time.
- The `language-tools/` submodule pin — bumped separately, since it
  moves the parity baseline for every suite at once.
- `notes/` — local, gitignored.
- Broad refactors bundled with a fix. Keep PRs to one concern.

## Commits and comments

- Imperative mood, lowercase, one concise line. Body when the "why"
  isn't obvious from the diff.
- Comments explain the code in its own terms. Don't make an issue or PR
  number the substance of a comment — it stops meaning anything once
  the tracker moves.
- Rebase on `main` rather than merging it in.

## Reporting bugs

A `.svelte` file that reproduces, the upstream `svelte-check --tsgo`
output on the same file, and ours. The delta between those two is the
bug; anything else is a guess.
