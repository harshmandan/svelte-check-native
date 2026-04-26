//! `{#key EXPR}` block emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Key.ts`.
//!
//! `{#key EXPR}body{/key}` re-creates the body when EXPR changes; from
//! the type checker's perspective, the body has the same scope as the
//! enclosing template — there's no introduced binding to check
//! against.
//!
//! **Status: dispatcher inline.** `lib.rs::emit_template_node`'s
//! `Node::KeyBlock(b)` arm walks the body via `emit_template_body`
//! directly. The KEY expression itself isn't currently emitted as a
//! ref — a future iteration could splice it into a `(EXPR);` line so
//! tsgo type-checks it like other interpolations.
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `Key.ts` should land here.
