//! Comment-node handling.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Comment.ts`.
//!
//! Upstream's handler erases comment bytes and threads "leading" /
//! "trailing" comments onto adjacent element nodes so the overlay's
//! identifier-position blame accounts for source comments. The
//! transformation is editor-tooling-driven (hover positions stay
//! correct in the IDE).
//!
//! **We don't need this.** Our overlay is built structurally rather
//! than by overwriting source bytes — comments simply aren't emitted
//! to the overlay in the first place. See `lib.rs::emit_template_node`'s
//! `Node::Comment(_)` arm: dispatcher no-op.
//!
//! `<!-- @component -->` is consumed at the svelte2tsx transform stage —
//! upstream's `nodes/ComponentDocumentation.ts` strips the tag and emits
//! the text as a leading JSDoc on the default-export component (IDE hover
//! only). It carries no type-check-surface effect, so we emit nothing.
//! `<!-- @hmr-keep -->` is a compiler/HMR-runtime concern, likewise
//! irrelevant to type checking.
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `Comment.ts` should land here.
