//! Semantic analysis passes over the Svelte AST.
//!
//! Populates a `SemanticModel` with: detected runes, prop destructures,
//! store subscriptions, `bind:` targets, SvelteKit route role, and — critically
//! — a `VoidRefRegistry` collecting every synthesized name that the emit crate
//! will need to reference. This replaces the ad-hoc per-feature `void x;`
//! emission scattered through `-rs`'s transformer (12 of 33 bugs).
//!
//! All passes share a single `Visitor` walk of the AST. One pass, many
//! collectors.

// Tests are allowed to panic loudly on setup failures.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod rune;
pub mod template_walker;
pub mod void_refs;

pub use rune::{RuneCall, RuneKind, find_runes};
pub use template_walker::{BindThisTarget, TemplateSummary, walk_template};
pub use void_refs::VoidRefRegistry;
