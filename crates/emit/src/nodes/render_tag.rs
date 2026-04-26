//! `{@render foo(x)}` snippet-render tag (Svelte 5+).
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/RenderTag.ts`.
//!
//! Upstream emits `;__sveltets_2_ensureSnippet(foo(x));` so tsgo:
//!   1. Type-checks the call's arguments against `foo`'s declared
//!      `Snippet<[…]>` parameter shape.
//!   2. Validates that `foo` IS a snippet (the `__sveltets_2_ensureSnippet`
//!      ambient constrains its arg to `Snippet<…> | undefined`).
//!
//! **Status: feature gap.** We currently treat `{@render}` as a
//! dispatcher no-op in `nodes/mustache_tag.rs::emit_interpolation`.
//! Snippet-typing TS2345s on `{@render}` calls don't fire — a real
//! parity concern as Svelte 5 codebases adopt snippets widely.
//!
//! Implementation when landed: route via a new
//! `InterpolationKind::AtRender` from the parser, emit through a
//! `__svn_ensure_snippet(EXPR)` helper that mirrors upstream's
//! ambient.
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `RenderTag.ts` should land here.
