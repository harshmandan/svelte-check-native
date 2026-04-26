//! Text-node handling.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Text.ts`.
//!
//! Upstream replaces text-node content with whitespace-only padding (or
//! a single space) inside elements so the overlay output stays
//! uncluttered while preserving line/column geometry for editor hover
//! UX.
//!
//! **We don't need a handler.** Our overlay is built bottom-up by
//! emitting only the structurally-significant nodes (script body,
//! interpolations, component calls, `{#each}`/`{#if}`/etc. wrappers);
//! text content is never spliced into the overlay in the first place.
//! See `lib.rs::emit_template_node`'s `Node::Text(_)` arm — it's a
//! dispatcher no-op by construction.
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `Text.ts` should land here when looking for our
//! equivalent.
