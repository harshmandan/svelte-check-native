//! Component-level JSDoc / `@component` comment handling.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ComponentDocumentation.ts`.
//!
//! **Status: feature gap.**
//!
//! Upstream's `ComponentDocumentation` extracts `<!-- @component -->`
//! HTML comments and `/** ... */` JSDoc blocks attached to the
//! default-exported component. The extracted text is attached to the
//! emitted `__sveltets_2_isomorphic_component(...)` call as a JSDoc
//! comment so the LSP can show component-level documentation in hover
//! tooltips.
//!
//! We don't extract these. Reasons:
//!
//! 1. We're CLI-only — no LSP consumer needs the doc text.
//! 2. tsgo doesn't surface JSDoc comments on declarations through any
//!    diagnostic surface we report on, so the comments would be
//!    write-only.
//!
//! If LSP support lands and component-level docs become a real
//! consumer, the natural place to add this is in
//! [`crate::default_export::emit_default_export_declarations_ts`] —
//! emit a leading JSDoc block above the
//! `const __svn_component_default: $$IsomorphicComponent = null as any;`
//! line.
//!
//! This file exists for parity navigation.
