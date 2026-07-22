//! # `svn-enhance` — tsgo-mode enhancements that diverge from upstream
//!
//! This crate is the isolated home for checks that go **beyond** what
//! `svelte-check --tsgo` can do, so that we match the behaviour of the
//! **default** `svelte-check` (the in-process language-server engine)
//! rather than the reduced `--tsgo` command surface we otherwise mirror.
//!
//! ## Why it exists — and why it is temporary
//!
//! Upstream's default engine owns an in-process `LanguageServiceHost` and
//! can install a custom module-resolution host (`svelte-sys` /
//! `DocumentSnapshot`) that resolves `.svelte` files and strips svelte's
//! `declare module '*.svelte'` wildcard, so a missing `.svelte` import
//! surfaces as `TS2307`. The `--tsgo` path (which we mirror) drives tsgo
//! as a subprocess and **cannot** alter its module resolution, so it falls
//! back to that wildcard — which resolves every `.svelte` specifier,
//! including missing ones, to `any` and swallows the error.
//!
//! The upstream maintainer states the constraint directly on
//! `sveltejs/language-tools#2733`:
//!
//! > "The main problem is that there is no way for us to alter the module
//! > resolution to resolve Svelte files."
//!
//! That capability is expected to arrive as a stable TypeScript-Go
//! resolution API only **after TypeScript 7 ships**. Once it lands and
//! `svelte-check --tsgo` resolves `.svelte` natively, everything in this
//! crate becomes redundant.
//!
//! ## Removal is mechanical
//!
//! Every callsite in the core pipeline is tagged `// TSGO-ENHANCEMENT`
//! (the analogue of the Svelte-4 layer's `// SVELTE-4-COMPAT` marker).
//! When upstream catches up: delete this crate, drop the `svn-enhance`
//! dependency, and `grep -r TSGO-ENHANCEMENT` to remove the (few) marked
//! callsites. Nothing in the core crates depends on this one's types.

mod missing_svelte_imports;

pub use missing_svelte_imports::{EnhancementDiagnostic, missing_svelte_import_diagnostics};
