# experiments/

Work that answered a question rather than shipping a feature.

Nothing here is built by `cargo test --workspace`, referenced by the CLI, or
covered by the release process. Each subdirectory declares its own
`[workspace]` so the root build never reaches it. Deleting any of them costs
nothing but the write-up.

The rule for landing something here rather than leaving it on a branch: it
produced a measurement or a bug that outlives the code. If the code is the
point, it belongs in `crates/`. If the conclusion is the point, it belongs
here with the evidence attached.

| experiment | question it answered |
|---|---|
| [`svelte-lsp-spike/`](svelte-lsp-spike/README.md) | Can a Svelte language server be built on tsgo without holding a whole-workspace TypeScript program in memory? |

## svelte-lsp-spike

Short answer: yes, and by a wide margin — but it is not a product.

Scoping tsgo to the open file's import closure instead of the workspace gives
the same diagnostics and the same hover answers at roughly a third of the
memory, with warm hovers around 1 ms against 245 ms. Full numbers, method, and
the parity gates are in its own README.

It also earned its keep in bugs. Six things only surfaced once something was
actually running, none of which the preceding measurements had predicted:

1. Composite projects make narrowing impossible — every `.ts` a component
   imports raises TS6307 when you deliberately do not list it.
2. Ambient declarations cannot be reached by following imports, so narrowing
   drops them and produces confident, wrong diagnostics.
3. "Every `.d.ts`" is far too many ambients — SvelteKit's generated per-route
   `$types.d.ts` are ordinary modules, and listing them drags the app back in.
4. Syncing and pulling diagnostics file-by-file costs a full re-check per file;
   tsgo declares `interFileDependencies`.
5. Hint-severity diagnostics split the two surfaces — the CLI drops them, a
   language server must not.
6. Over-encoding a file URI silently loses every diagnostic for SvelteKit
   route directories, which use `(group)` and `[param]` with different rules.

One of those turned into a real parity bug in the shipping CLI, found because
the language server surfaced a class of diagnostic the batch path never
computes: a `<template lang="pug">` component reported unused imports upstream
does not report. Fixed and released separately.

**Status: closed.** The design is validated and the findings are recorded. The
gap between this and something installable is not the type-checking pipeline —
that part works — it is a total source map (rename cannot use a partial one
without silently skipping occurrences) and Svelte-aware completion, which is
where the existing implementations spend their effort too.
