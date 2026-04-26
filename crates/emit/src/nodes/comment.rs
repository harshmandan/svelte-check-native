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
//! `<!-- @hmr-keep -->` and `<!-- @component -->` (Svelte's recognised
//! comment directives) aren't emit-layer concerns either — they're
//! handled at parse / runtime stages.
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `Comment.ts` should land here.
