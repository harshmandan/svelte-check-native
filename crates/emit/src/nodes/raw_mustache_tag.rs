//! `{@html EXPR}` raw-HTML tag.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/RawMustacheTag.ts`.
//!
//! Upstream emits the expression as a bare statement: `{@html foo}`
//! becomes `;foo;` so tsgo type-checks `foo` against the surrounding
//! scope.
//!
//! **Status: feature gap.** We currently treat `{@html}` as a no-op in
//! `nodes/mustache_tag.rs::emit_interpolation`. The expression is not
//! type-checked and any TS errors inside it are lost. `Node::Interpolation`
//! routes by `InterpolationKind`; a future `RawHtml` kind on the
//! parser side plus a handler here would restore parity.
//!
//! Implementation sketch when we land it:
//! ```ignore
//! pub(crate) fn emit_raw_html(
//!     buf: &mut EmitBuffer,
//!     source: &str,
//!     interp: &svn_parser::Interpolation,
//!     depth: usize,
//! ) {
//!     // ... emit `(EXPR);\n` with a TokenMapEntry on the expression
//! }
//! ```
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `RawMustacheTag.ts` should land here.
