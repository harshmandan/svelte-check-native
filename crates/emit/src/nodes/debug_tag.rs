//! `{@debug a, b}` debug tag.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/DebugTag.ts`.
//!
//! Upstream emits each comma-separated identifier as a bare statement
//! (`{@debug a, b}` → `;a;b;`) so tsgo type-checks each identifier
//! against the surrounding scope.
//!
//! **Status: feature gap.** We currently treat `{@debug}` as a
//! dispatcher no-op in `nodes/mustache_tag.rs::emit_interpolation`.
//! TS2304 ("cannot find name") on `{@debug typo}` doesn't fire.
//!
//! Implementation when landed: route via a new
//! `InterpolationKind::AtDebug` from the parser, walk the
//! comma-separated identifier list, emit one bare statement per
//! identifier with a TokenMapEntry pointing at the source position.
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `DebugTag.ts` should land here.
